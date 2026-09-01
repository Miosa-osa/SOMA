//! Idempotent cleanup of everything a launch created, plus recovery from a crashed launcher.

use std::{
    fs, io,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use crate::{
    cgroup::{CgroupError, CgroupLeaf},
    process::{send_signal, wait_exit},
};

pub use disposition::{Disposition, Residual, ResidualKind};

// What a pass can leave behind, and how that reads, is beside this file. This one owns the
// releasing; that one owns the vocabulary a caller is answered in, which is the part other
// crates match on and print.
mod disposition;

const DRAIN_BACKOFF: Duration = Duration::from_millis(5);
/// How long a dropped ledger may spend releasing what it still owns.
const DROP_DEADLINE: Duration = Duration::from_secs(2);

/// The durable form of a ledger, enough to recover after the launcher itself died.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerRecord {
    pub leaf: String,
    pub jail_root: PathBuf,
    pub pid: Option<i32>,
}

/// Ownership of every resource one launch creates, recorded before the effect.
#[derive(Debug)]
pub struct JailLedger {
    leaf: String,
    jail_root: PathBuf,
    cgroup: Option<CgroupLeaf>,
    jail_root_created: bool,
    pid: Option<i32>,
    pidfd: Option<OwnedFd>,
    reaped: bool,
    disposition: Option<Disposition>,
}

fn errno_of(error: &io::Error) -> i32 {
    error.raw_os_error().unwrap_or(0)
}

fn remove_directory(path: &Path) -> Result<(), i32> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(errno_of(&error)),
    }
}

fn drain_and_remove(leaf: &CgroupLeaf, deadline: Instant, residuals: &mut Vec<Residual>) -> bool {
    let _ = leaf.kill_all();
    while leaf.populated().unwrap_or(false) && Instant::now() < deadline {
        thread::sleep(DRAIN_BACKOFF);
    }
    match leaf.remove(deadline) {
        Ok(()) => true,
        Err(CgroupError::Remove(errno)) => {
            residuals.push(Residual {
                kind: ResidualKind::Cgroup,
                errno,
            });
            false
        }
        Err(_) => {
            residuals.push(Residual {
                kind: ResidualKind::Cgroup,
                errno: 0,
            });
            false
        }
    }
}

impl JailLedger {
    pub(crate) fn new(leaf: String, jail_root: PathBuf) -> Self {
        Self {
            leaf,
            jail_root,
            cgroup: None,
            jail_root_created: false,
            pid: None,
            pidfd: None,
            reaped: false,
            disposition: None,
        }
    }

    pub(crate) fn record_cgroup(&mut self, leaf: CgroupLeaf) {
        self.cgroup = Some(leaf);
    }

    pub(crate) fn record_jail_root(&mut self) {
        self.jail_root_created = true;
    }

    pub(crate) fn record_child(&mut self, pid: i32, pidfd: OwnedFd) {
        self.pid = Some(pid);
        self.pidfd = Some(pidfd);
    }

    pub(crate) fn record_reaped(&mut self) {
        self.reaped = true;
    }

    pub(crate) fn cgroup(&self) -> Option<&CgroupLeaf> {
        self.cgroup.as_ref()
    }

    pub(crate) fn pidfd(&self) -> Option<BorrowedFd<'_>> {
        self.pidfd.as_ref().map(AsFd::as_fd)
    }

    #[must_use]
    pub fn record(&self) -> LedgerRecord {
        LedgerRecord {
            leaf: self.leaf.clone(),
            jail_root: self.jail_root.clone(),
            pid: self.pid,
        }
    }

    /// The last disposition, if [`Self::reconcile`] ran.
    #[must_use]
    pub fn disposition(&self) -> Option<&Disposition> {
        self.disposition.as_ref()
    }

    /// Kills through the pidfd, reaps, removes the leaf, and removes the jail root.
    ///
    /// Repeating the call retries only what is still owned, so it is idempotent.
    pub fn reconcile(&mut self, deadline: Instant) -> Disposition {
        let mut residuals = Vec::new();
        if let Some(pidfd) = self.pidfd.as_ref().map(AsFd::as_fd)
            && !self.reaped
        {
            let _ = send_signal(pidfd, libc::SIGKILL);
            if let Some(leaf) = &self.cgroup {
                let _ = leaf.kill_all();
            }
            match wait_exit(pidfd, deadline) {
                Ok(_) | Err(crate::process::WaitError::AlreadyReaped) => self.reaped = true,
                Err(crate::process::WaitError::Timeout) => {
                    residuals.push(Residual {
                        kind: ResidualKind::Process,
                        errno: libc::ETIMEDOUT,
                    });
                }
                Err(crate::process::WaitError::Errno(errno)) => {
                    residuals.push(Residual {
                        kind: ResidualKind::Process,
                        errno,
                    });
                }
            }
        }
        if self.reaped {
            self.pidfd = None;
        }
        if let Some(leaf) = &self.cgroup
            && drain_and_remove(leaf, deadline, &mut residuals)
        {
            self.cgroup = None;
        }
        if self.jail_root_created {
            match remove_directory(&self.jail_root) {
                Ok(()) => self.jail_root_created = false,
                Err(errno) => residuals.push(Residual {
                    kind: ResidualKind::JailRoot,
                    errno,
                }),
            }
        }
        let disposition = Disposition::from_residuals(residuals);
        self.disposition = Some(disposition.clone());
        disposition
    }

    /// Recovers the resources named by a record whose launcher is gone.
    ///
    /// The process, if any, is killed through `cgroup.kill` because no pidfd survives a crash.
    #[must_use]
    pub fn recover(cgroup_root: &Path, record: &LedgerRecord, deadline: Instant) -> Disposition {
        let mut residuals = Vec::new();
        match CgroupLeaf::open_existing(cgroup_root, &record.leaf) {
            Ok(leaf) => {
                drain_and_remove(&leaf, deadline, &mut residuals);
            }
            Err(CgroupError::Open(errno)) if errno == libc::ENOENT => {}
            Err(CgroupError::Open(errno)) => {
                residuals.push(Residual {
                    kind: ResidualKind::Cgroup,
                    errno,
                });
            }
            Err(_) => residuals.push(Residual {
                kind: ResidualKind::Cgroup,
                errno: 0,
            }),
        }
        if let Err(errno) = remove_directory(&record.jail_root) {
            residuals.push(Residual {
                kind: ResidualKind::JailRoot,
                errno,
            });
        }
        Disposition::from_residuals(residuals)
    }
}

impl Drop for JailLedger {
    /// Best-effort release of anything still owned, so a dropped handle never leaks a process
    /// or a leaf; whatever remains is recoverable later through [`Self::recover`].
    fn drop(&mut self) {
        if self.pidfd.is_some() || self.cgroup.is_some() || self.jail_root_created {
            let _ = self.reconcile(Instant::now() + DROP_DEADLINE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconciling_an_empty_ledger_is_released_and_idempotent() {
        let mut ledger =
            JailLedger::new("leaf".into(), PathBuf::from("/nonexistent/soma-jail-test"));
        assert_eq!(ledger.reconcile(Instant::now()), Disposition::Released);
        assert_eq!(ledger.reconcile(Instant::now()), Disposition::Released);
        assert_eq!(ledger.record().pid, None);
    }

    #[test]
    fn dispositions_display_their_residuals() {
        let incomplete = Disposition::Incomplete {
            residuals: vec![Residual {
                kind: ResidualKind::Cgroup,
                errno: 16,
            }],
        };
        assert_eq!(incomplete.to_string(), "incomplete: Cgroup(errno 16)");
        assert!(Disposition::Released.is_released());
    }
}
