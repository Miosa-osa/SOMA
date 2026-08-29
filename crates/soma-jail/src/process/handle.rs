//! The parent's handle on one jailed process.

use std::time::Instant;

use super::wait::{SignalError, WaitError, send_signal, wait_exit};
use crate::{
    cgroup::CgroupLeaf,
    evidence::{ExitReason, JailEvidence},
    reconcile::{Disposition, JailLedger},
};

/// One launched jail: pidfd ownership, evidence, and the ledger that cleans it up.
#[derive(Debug)]
pub struct JailHandle {
    ledger: JailLedger,
    pid: i32,
    evidence: JailEvidence,
}

impl JailHandle {
    pub(crate) fn new(ledger: JailLedger, pid: i32, evidence: JailEvidence) -> Self {
        Self {
            ledger,
            pid,
            evidence,
        }
    }

    /// The child's PID in the launcher's namespace; informational only, never used to signal.
    #[must_use]
    pub fn pid(&self) -> i32 {
        self.pid
    }

    #[must_use]
    pub fn evidence(&self) -> &JailEvidence {
        &self.evidence
    }

    #[must_use]
    pub fn ledger(&self) -> &JailLedger {
        &self.ledger
    }

    /// The leaf this process runs in.
    ///
    /// # Panics
    ///
    /// Never for a handle returned by `launch`, which always records its leaf before the child.
    #[must_use]
    pub fn cgroup(&self) -> &CgroupLeaf {
        self.ledger
            .cgroup()
            .expect("a launched jail owns its cgroup leaf")
    }

    /// Waits for the child through its pidfd and records the exit reason in the evidence.
    ///
    /// # Errors
    ///
    /// Returns [`WaitError::Timeout`] when the deadline passes first.
    pub fn wait(&mut self, deadline: Instant) -> Result<ExitReason, WaitError> {
        if let Some(exit) = self.evidence.exit {
            return Ok(exit);
        }
        let pidfd = self.ledger.pidfd().ok_or(WaitError::AlreadyReaped)?;
        let exit = wait_exit(pidfd, deadline)?;
        self.evidence.exit = Some(exit);
        self.evidence.oom_kills = self
            .ledger
            .cgroup()
            .and_then(|leaf| leaf.oom_kills().ok())
            .unwrap_or(0);
        self.ledger.record_reaped();
        Ok(exit)
    }

    /// Sends `signal` through the pidfd.
    ///
    /// # Errors
    ///
    /// Returns [`SignalError::Gone`] once the process has exited, even if its PID was reused.
    pub fn signal(&self, signal: i32) -> Result<(), SignalError> {
        let pidfd = self.ledger.pidfd().ok_or(SignalError::Gone)?;
        send_signal(pidfd, signal)
    }

    /// Sends `SIGKILL` through the pidfd.
    ///
    /// # Errors
    ///
    /// Returns [`SignalError::Gone`] once the process has exited.
    pub fn kill(&self) -> Result<(), SignalError> {
        self.signal(libc::SIGKILL)
    }

    /// Kills, reaps, and removes everything, returning the disposition and final evidence.
    #[must_use]
    pub fn reconcile(mut self, deadline: Instant) -> (Disposition, JailEvidence) {
        if self.evidence.exit.is_none() {
            let _ = self.kill();
            if let Ok(exit) = self.wait(deadline) {
                self.evidence.exit = Some(exit);
            }
        }
        let disposition = self.ledger.reconcile(deadline);
        (disposition, self.evidence)
    }
}
