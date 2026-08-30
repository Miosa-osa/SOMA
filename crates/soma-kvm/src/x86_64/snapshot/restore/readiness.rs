//! The authenticated readiness transition of one restored Instance.
//!
//! `Restored::ready` is the last ordered step of a restore, and it is the only place the
//! machine may be called ready. Nothing outside the restore consumes that transition yet, so a
//! refused or never-attempted readiness records a fact rather than withholding execution or
//! network activation. It consumes a receipt minted from this restore's own
//! single-use challenge and bound to the exact snapshot, the exact published launch authority,
//! and one authenticated repaired guest session, so no caller can advance the typestate by
//! asserting readiness.

use super::{Restored, SnapshotError};
use crate::snapshot::readiness::{
    ReadinessChallenge, ReadinessDemand, ReadinessReceipt, ReadinessRefusal, RestoredIdentity,
};
use crate::snapshot::restore::RestoreStep;
use crate::virtio::{EntropyBackend as _, OsEntropy};

impl Restored {
    /// The evidence one guest-session owner must mint before this Instance can be ready.
    ///
    /// Returns `None` before the fresh launch authority is published and after the single-use
    /// challenge has been spent.
    #[must_use]
    pub fn readiness_demand(&self) -> Option<ReadinessDemand<'_>> {
        let (launch, session) = self.launch.get().filter(|_| !self.spent.get())?;
        let identity = RestoredIdentity::new(self.facts.snapshot, launch, session);
        Some(ReadinessDemand::new(&self.readiness, identity))
    }

    /// Consumes the authenticated readiness receipt and completes the restore.
    ///
    /// The receipt must authenticate against this restore's own challenge and bind the exact
    /// snapshot it came from, the exact launch authority it published, and the live transcript
    /// of the guest session that completed authenticated repair and the fixed readiness probe.
    /// Its Instance and Launch operation must be the ones the published page itself carries. The challenge is taken before anything is
    /// verified, so a refused receipt cannot be retried against this Instance.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError::Readiness`] when no launch authority has been published, the
    /// challenge is already spent, or the receipt does not authenticate, and the ordering
    /// violation when the machine has not resumed.
    pub fn ready(&self, receipt: &ReadinessReceipt) -> Result<(), SnapshotError> {
        let (launch, session) = self.launch.get().ok_or(ReadinessRefusal::Unpublished)?;
        if self.spent.replace(true) {
            return Err(ReadinessRefusal::Spent.into());
        }
        let identity = RestoredIdentity::new(self.facts.snapshot, launch, session);
        self.readiness.accepts(&identity, receipt)?;
        self.step(RestoreStep::AuthenticatedRepairAndReadiness)
    }
}

/// Samples the fresh secret this restore will require in its readiness receipt.
pub(super) fn sample_challenge() -> Result<ReadinessChallenge, SnapshotError> {
    let mut bytes = [0_u8; 32];
    OsEntropy::open()
        .and_then(|mut source| source.fill(&mut bytes))
        .map_err(|_| ReadinessRefusal::Unavailable)?;
    Ok(ReadinessChallenge::adopt(bytes)?)
}
