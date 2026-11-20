//! The broker's exclusive hold on its state root.
//!
//! One broker owns a state root at a time. A second one on the same root would hand out the
//! same addresses and fight over the same ledger, so a starting broker takes this lock first
//! and keeps it for the life of the process.

use std::time::Duration;

use crate::{Error, Step};

/// An exclusive advisory lock on a broker's state root, held for the life of the process.
pub(super) struct InstanceLock {
    _file: std::fs::File,
}

/// How long a starting broker waits for a previous one to release the state root.
///
/// A stop completes before a start under systemd, but a process that outlives its stop (for
/// instance one still tearing down) would otherwise make the new one fail at once and crash-loop.
/// The wait is bounded so a genuinely second broker still fails, and the wait is logged.
const LOCK_WAIT: Duration = Duration::from_secs(30);

impl InstanceLock {
    pub(super) fn acquire(root: &std::path::Path) -> Result<Self, Error> {
        Self::acquire_within(root, LOCK_WAIT)
    }

    fn acquire_within(root: &std::path::Path, wait: Duration) -> Result<Self, Error> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("netd.lock"))
            .map_err(|error| Error::io(Step::LedgerOpen, &error))?;
        let deadline = std::time::Instant::now() + wait;
        let mut announced = false;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(std::fs::TryLockError::WouldBlock) => {
                    if std::time::Instant::now() >= deadline {
                        return Err(Error::Kernel {
                            step: Step::LedgerOpen,
                            errno: libc::EWOULDBLOCK,
                        });
                    }
                    if !announced {
                        eprintln!(
                            "soma-netd: another broker holds {}; waiting up to {} s for it to exit",
                            root.join("netd.lock").display(),
                            wait.as_secs()
                        );
                        announced = true;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(std::fs::TryLockError::Error(error)) => {
                    return Err(Error::io(Step::LedgerOpen, &error));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::InstanceLock;

    #[test]
    fn a_second_broker_on_the_same_state_root_is_refused_after_a_bounded_wait() {
        let root = tempfile::tempdir().expect("state root");
        let first = InstanceLock::acquire(root.path()).expect("first broker");
        let started = std::time::Instant::now();
        assert!(
            InstanceLock::acquire_within(root.path(), Duration::from_millis(300)).is_err(),
            "the lock is exclusive"
        );
        assert!(started.elapsed() >= Duration::from_millis(300), "it waited");
        drop(first);
        InstanceLock::acquire(root.path()).expect("released when the first ends");
    }

    #[test]
    fn a_broker_that_is_still_exiting_does_not_crash_loop_its_successor() {
        let root = tempfile::tempdir().expect("state root");
        let old = InstanceLock::acquire(root.path()).expect("the old broker");
        let releasing = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            drop(old);
        });
        InstanceLock::acquire_within(root.path(), Duration::from_secs(5))
            .expect("the new broker gets the lock once the old one is gone");
        releasing.join().expect("old broker thread");
    }
}
