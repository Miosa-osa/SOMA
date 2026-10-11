//! Changing a running machine's CPU share, and giving a machine that was not built inside a jail
//! a leaf of its own to be limited in.
//!
//! The jail writes every limit before a process joins its leaf and never writes them again. A
//! live resize writes `cpu.max` alone, reads it back, and reports what the kernel holds, so the
//! caller never claims a share the kernel did not accept.

use std::{fs, path::Path};

use super::{CgroupError, CgroupLeaf, errno_of, parse_max, read, require_cgroup2};
use crate::spec::CpuMax;

/// The smallest period the kernel accepts, in microseconds.
const MIN_PERIOD_US: u64 = 1_000;
/// The largest period the kernel accepts, in microseconds.
const MAX_PERIOD_US: u64 = 1_000_000;
/// The smallest quota the kernel accepts, in microseconds.
const MIN_QUOTA_US: u64 = 1_000;

impl CgroupLeaf {
    /// Creates, or reopens, a leaf that limits CPU and nothing else.
    ///
    /// A machine host that was not started inside a jail has no leaf, so a live CPU change gives
    /// it one: the host moves itself in, and every thread it runs follows. No other limit is
    /// written, so the leaf cannot make the machine worse off than it was.
    ///
    /// # Errors
    ///
    /// Returns a [`CgroupError`] when the root is not cgroup2, the cpu controller is missing or
    /// undelegated, or the leaf cannot be created or opened.
    pub fn create_cpu_only(root: &Path, leaf: &str) -> Result<Self, CgroupError> {
        require_cgroup2(root)?;
        let controllers = read(root, "cgroup.controllers")?;
        let delegated = read(root, "cgroup.subtree_control")?;
        if !controllers.split_whitespace().any(|name| name == "cpu") {
            return Err(CgroupError::ControllerUnavailable("cpu"));
        }
        if !delegated.split_whitespace().any(|name| name == "cpu") {
            return Err(CgroupError::ControllerNotDelegated("cpu"));
        }
        match fs::create_dir(root.join(leaf)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(CgroupError::Create(errno_of(&error))),
        }
        Self::open_existing(root, leaf)
    }

    /// Moves a process, with every thread it has, into this leaf.
    ///
    /// # Errors
    ///
    /// Returns [`CgroupError::Write`] when the kernel refuses the move.
    pub fn attach(&self, pid: u32) -> Result<(), CgroupError> {
        self.write("cgroup.procs", &pid.to_string())
    }

    /// Sets `cpu.max` and returns what the kernel reports afterwards.
    ///
    /// # Errors
    ///
    /// Returns [`CgroupError::Readback`] for a quota or period the kernel would not accept, and
    /// a write or read failure otherwise.
    pub fn set_cpu_max(&self, cpu: CpuMax) -> Result<CpuMax, CgroupError> {
        if cpu.quota_us < MIN_QUOTA_US || !(MIN_PERIOD_US..=MAX_PERIOD_US).contains(&cpu.period_us)
        {
            return Err(CgroupError::Readback {
                file: "cpu.max",
                expected: "quota >= 1000 and period 1000..=1000000".to_owned(),
                found: format!("{} {}", cpu.quota_us, cpu.period_us),
            });
        }
        self.write("cpu.max", &format!("{} {}", cpu.quota_us, cpu.period_us))?;
        self.cpu_max()
    }

    /// Reads `cpu.max` back from the kernel.
    ///
    /// # Errors
    ///
    /// Returns [`CgroupError::Read`] or [`CgroupError::Readback`] for an unparsable value.
    pub fn cpu_max(&self) -> Result<CpuMax, CgroupError> {
        let text = self.read("cpu.max")?;
        let bad = || CgroupError::Readback {
            file: "cpu.max",
            expected: "quota period".to_owned(),
            found: text.clone(),
        };
        let (quota, period) = text.split_once(' ').ok_or_else(bad)?;
        Ok(CpuMax {
            quota_us: parse_max("cpu.max", quota)?,
            period_us: parse_max("cpu.max", period)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf() -> (tempfile::TempDir, CgroupLeaf) {
        let root = tempfile::tempdir().expect("root");
        fs::create_dir(root.path().join("soma-x")).expect("leaf");
        fs::write(root.path().join("soma-x/cpu.max"), "100000 100000").expect("seed");
        let leaf = CgroupLeaf::open_existing(root.path(), "soma-x").expect("open");
        (root, leaf)
    }

    #[test]
    fn a_cpu_change_is_written_and_read_back() {
        let (_root, leaf) = leaf();
        let applied = leaf
            .set_cpu_max(CpuMax {
                quota_us: 50_000,
                period_us: 100_000,
            })
            .expect("set");
        assert_eq!(applied.quota_us, 50_000);
        assert_eq!(leaf.cpu_max().expect("read"), applied);
    }

    #[test]
    fn a_quota_or_period_the_kernel_would_refuse_never_reaches_it() {
        let (root, leaf) = leaf();
        for (quota_us, period_us) in [(0, 100_000), (999, 100_000), (50_000, 999), (1, 2_000_000)] {
            assert!(
                leaf.set_cpu_max(CpuMax {
                    quota_us,
                    period_us
                })
                .is_err()
            );
        }
        let kept = fs::read_to_string(root.path().join("soma-x/cpu.max")).expect("file");
        assert_eq!(
            kept, "100000 100000",
            "a refused change must not be written"
        );
    }

    #[test]
    fn a_leaf_without_a_cpu_file_is_a_read_failure_not_a_panic() {
        let root = tempfile::tempdir().expect("root");
        fs::create_dir(root.path().join("soma-y")).expect("leaf");
        let leaf = CgroupLeaf::open_existing(root.path(), "soma-y").expect("open");
        assert!(leaf.cpu_max().is_err());
    }
}
