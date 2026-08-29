//! One cgroup v2 leaf per jail: creation, limits, membership, kill, and removal.
//!
//! Every failure is typed; an unavailable or undelegated controller is an error, never a skip.

mod error;
mod files;

use std::{
    fs, io,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

pub use error::CgroupError;

use self::files::{errno_of, io_max_line, parse_max, read, readback, require_cgroup2};
use crate::spec::{CgroupLimits, CpuMax, LeafName};

const REMOVE_BACKOFF: Duration = Duration::from_millis(5);

/// What the leaf reports after the limits were written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CgroupReadback {
    pub memory_max: u64,
    pub swap_max: u64,
    pub oom_group: bool,
    pub cpu_max: CpuMax,
    pub pids_max: u32,
    pub io_max: Option<String>,
}

/// One owned leaf directory under the caller-provided cgroup2 root.
#[derive(Debug)]
pub struct CgroupLeaf {
    path: PathBuf,
    leaf: String,
    dir: OwnedFd,
}

impl CgroupLeaf {
    /// Creates the leaf and writes every limit before any process joins it.
    ///
    /// # Errors
    ///
    /// Returns a [`CgroupError`] when the root is not cgroup2, a needed controller is missing or
    /// undelegated, the leaf exists, or a limit cannot be written.
    pub fn create(
        root: &Path,
        leaf: &LeafName,
        limits: &CgroupLimits,
    ) -> Result<Self, CgroupError> {
        require_cgroup2(root)?;
        let mut needed = vec!["cpu", "memory", "pids"];
        if limits.io_max.is_some() {
            needed.push("io");
        }
        let controllers = read(root, "cgroup.controllers")?;
        let delegated = read(root, "cgroup.subtree_control")?;
        for name in needed {
            if !controllers.split_whitespace().any(|found| found == name) {
                return Err(CgroupError::ControllerUnavailable(name));
            }
            if !delegated.split_whitespace().any(|found| found == name) {
                return Err(CgroupError::ControllerNotDelegated(name));
            }
        }
        let path = root.join(leaf.as_str());
        fs::create_dir(&path).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                CgroupError::AlreadyExists
            } else {
                CgroupError::Create(errno_of(&error))
            }
        })?;
        let created = Self::open_at(path, leaf.as_str().to_owned())?;
        created.write_limits(limits)?;
        Ok(created)
    }

    /// Opens an existing leaf for recovery without touching its limits.
    ///
    /// # Errors
    ///
    /// Returns [`CgroupError::Open`] when the leaf does not exist.
    pub fn open_existing(root: &Path, leaf: &str) -> Result<Self, CgroupError> {
        Self::open_at(root.join(leaf), leaf.to_owned())
    }

    fn open_at(path: PathBuf, leaf: String) -> Result<Self, CgroupError> {
        let dir = fs::File::open(&path).map_err(|error| CgroupError::Open(errno_of(&error)))?;
        Ok(Self {
            path,
            leaf,
            dir: dir.into(),
        })
    }

    fn write_limits(&self, limits: &CgroupLimits) -> Result<(), CgroupError> {
        self.write("memory.max", &limits.memory_max_bytes.to_string())?;
        self.write("memory.swap.max", "0")?;
        self.write("memory.oom.group", "1")?;
        let CpuMax {
            quota_us,
            period_us,
        } = limits.cpu_max;
        self.write("cpu.max", &format!("{quota_us} {period_us}"))?;
        self.write("pids.max", &limits.pids_max.to_string())?;
        if let Some(io) = limits.io_max {
            self.write("io.max", &io_max_line(io))?;
        }
        Ok(())
    }

    fn write(&self, file: &'static str, value: &str) -> Result<(), CgroupError> {
        fs::write(self.path.join(file), value).map_err(|error| CgroupError::Write {
            file,
            errno: errno_of(&error),
        })
    }

    fn read(&self, file: &'static str) -> Result<String, CgroupError> {
        read(&self.path, file)
    }

    /// Reads every limit back from the kernel.
    ///
    /// # Errors
    ///
    /// Returns [`CgroupError::Read`] or [`CgroupError::Readback`] for an unparsable value.
    pub fn readback(&self) -> Result<CgroupReadback, CgroupError> {
        let cpu = self.read("cpu.max")?;
        let (quota, period) = cpu
            .split_once(' ')
            .ok_or_else(|| readback("cpu.max", "quota period", &cpu))?;
        let io_max = self.read("io.max").ok().filter(|value| !value.is_empty());
        Ok(CgroupReadback {
            memory_max: parse_max("memory.max", &self.read("memory.max")?)?,
            swap_max: parse_max("memory.swap.max", &self.read("memory.swap.max")?)?,
            oom_group: self.read("memory.oom.group")? == "1",
            cpu_max: CpuMax {
                quota_us: parse_max("cpu.max", quota)?,
                period_us: parse_max("cpu.max", period)?,
            },
            pids_max: u32::try_from(parse_max("pids.max", &self.read("pids.max")?)?)
                .map_err(|_| readback("pids.max", "u32", "overflow"))?,
            io_max,
        })
    }

    /// Reads the limits back and compares them with `limits`.
    ///
    /// # Errors
    ///
    /// Returns [`CgroupError::Readback`] naming the first mismatched file.
    pub fn verify(&self, limits: &CgroupLimits) -> Result<CgroupReadback, CgroupError> {
        let found = self.readback()?;
        let expect = |file: &'static str, ok: bool, expected: String, actual: String| {
            if ok {
                Ok(())
            } else {
                Err(CgroupError::Readback {
                    file,
                    expected,
                    found: actual,
                })
            }
        };
        let memory = limits.memory_max_bytes;
        expect(
            "memory.max",
            found.memory_max == memory,
            memory.to_string(),
            found.memory_max.to_string(),
        )?;
        expect(
            "memory.swap.max",
            found.swap_max == 0,
            "0".into(),
            found.swap_max.to_string(),
        )?;
        expect("memory.oom.group", found.oom_group, "1".into(), "0".into())?;
        let cpu = limits.cpu_max;
        expect(
            "cpu.max",
            found.cpu_max == cpu,
            format!("{cpu:?}"),
            format!("{:?}", found.cpu_max),
        )?;
        let pids = limits.pids_max;
        expect(
            "pids.max",
            found.pids_max == pids,
            pids.to_string(),
            found.pids_max.to_string(),
        )?;
        if let Some(io) = limits.io_max {
            let device = format!("{}:{}", io.major, io.minor);
            let present = found
                .io_max
                .as_deref()
                .is_some_and(|line| line.starts_with(&device));
            expect(
                "io.max",
                present,
                io_max_line(io),
                found.io_max.clone().unwrap_or_default(),
            )?;
        }
        Ok(found)
    }

    /// Whether `pid` (as seen by this process) is listed in `cgroup.procs`.
    ///
    /// # Errors
    ///
    /// Returns [`CgroupError::Read`] if the file cannot be read.
    pub fn contains(&self, pid: i32) -> Result<bool, CgroupError> {
        Ok(self
            .read("cgroup.procs")?
            .lines()
            .any(|line| line.trim() == pid.to_string()))
    }

    /// `memory.events` `oom_kill` count.
    ///
    /// # Errors
    ///
    /// Returns [`CgroupError::Read`] if the file cannot be read.
    pub fn oom_kills(&self) -> Result<u64, CgroupError> {
        let events = self.read("memory.events")?;
        Ok(events
            .lines()
            .find_map(|line| line.strip_prefix("oom_kill "))
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0))
    }

    /// Whether any process remains in the leaf.
    ///
    /// # Errors
    ///
    /// Returns [`CgroupError::Read`] if `cgroup.events` cannot be read.
    pub fn populated(&self) -> Result<bool, CgroupError> {
        Ok(self
            .read("cgroup.events")?
            .lines()
            .any(|line| line.trim() == "populated 1"))
    }

    /// Kills every process in the leaf through `cgroup.kill`.
    ///
    /// # Errors
    ///
    /// Returns [`CgroupError::Write`] if the kernel rejects the write.
    pub fn kill_all(&self) -> Result<(), CgroupError> {
        self.write("cgroup.kill", "1")
    }

    /// Removes the leaf, retrying `EBUSY` until `deadline` while the kernel drains it.
    ///
    /// # Errors
    ///
    /// Returns [`CgroupError::Remove`] with the last errno if the leaf still exists at the
    /// deadline.
    pub fn remove(&self, deadline: Instant) -> Result<(), CgroupError> {
        loop {
            match fs::remove_dir(&self.path) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => {
                    let errno = errno_of(&error);
                    if Instant::now() >= deadline || errno != libc::EBUSY {
                        return Err(CgroupError::Remove(errno));
                    }
                    thread::sleep(REMOVE_BACKOFF);
                }
            }
        }
    }

    #[must_use]
    pub fn exists(&self) -> bool {
        self.path.is_dir()
    }

    #[must_use]
    pub fn dir_fd(&self) -> BorrowedFd<'_> {
        self.dir.as_fd()
    }

    #[must_use]
    pub fn leaf_name(&self) -> &str {
        &self.leaf
    }
}
