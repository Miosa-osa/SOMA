//! The restored machine a caller drives: its facts, its remaining ordered steps, and the resume
//! that publishes fresh launch material.

use std::cell::Cell;

use super::super::error::SnapshotError;
use crate::snapshot::{
    Digest,
    readiness::{PageSession, ReadinessChallenge, ReadinessRefusal, page_session},
    restore::{RestoreSequence, RestoreStep},
};
use crate::x86_64::launch_page::LAUNCH_PAGE_SIZE;
use crate::x86_64::sandbox::SandboxMachine;

use super::RestoreFacts;

/// A restored machine and its remaining ordered steps.
pub struct Restored {
    /// The machine, ready for its fresh launch page.
    pub machine: SandboxMachine,
    /// What the snapshot said this machine is.
    pub facts: RestoreFacts,
    pub(super) sequence: Cell<RestoreSequence>,
    /// The fresh single-use secret this restore requires in its readiness receipt.
    pub(super) readiness: ReadinessChallenge,
    /// Whether one readiness attempt has already spent that challenge.
    pub(super) spent: Cell<bool>,
    /// The launch authority this restore published and the session that page binds, once it
    /// has published one.
    pub(super) launch: Cell<Option<(Digest, PageSession)>>,
}

impl Restored {
    /// Publishes the fresh launch material and resumes vCPU 0.
    ///
    /// # Errors
    ///
    /// Returns the ordering violation or the machine failure.
    pub fn resume(&mut self, page: &[u8; LAUNCH_PAGE_SIZE]) -> Result<(), SnapshotError> {
        self.step(RestoreStep::AttachFreshAuthority)?;
        self.machine.write_launch_page(page)?;
        let session = page_session(page).ok_or(ReadinessRefusal::Unbound)?;
        self.launch.set(Some((Digest::of(page), session)));
        self.machine.start()?;
        // The vsock restore queued a transport-reset event; delivering it now is what makes
        // the guest driver re-read the fresh context identifier before the agent connects.
        self.machine.wake_devices();
        self.step(RestoreStep::ResumeVcpu)
    }

    /// Whether every ordered step completed.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.sequence.get().is_ready()
    }

    pub(super) fn step(&self, step: RestoreStep) -> Result<(), SnapshotError> {
        let mut sequence = self.sequence.get();
        sequence.complete(step)?;
        self.sequence.set(sequence);
        Ok(())
    }
}
