//! How one Launch comes to hold a machine, either way it gets one.
//!
//! A Launch either claims a machine the pool prepared before the request arrived or restores one
//! of its own. The two arms differ only in where the machine came from: both end holding an
//! authenticated session, and both stamp the same milestone when the machine exists, so the two
//! remain comparable and a reader can tell them apart by the reported class rather than by
//! guessing from a duration.

use soma::{BackendFailure, BackendFailureKind, InstanceId, OperationId, PreparationClass};

use super::{
    KvmBackend,
    boot::boot_for,
    claim,
    prepared::PreparedGeneration,
    session::{Session, SessionError},
};

/// One sandbox that reached Ready, and how it came to exist.
pub(super) struct Started {
    /// Whether a prepared machine served this Launch or it built its own.
    pub(super) preparation: PreparationClass,
    /// The authenticated session over the machine.
    pub(super) session: Session,
    /// When this Launch finished producing a machine, in nanoseconds since it was accepted.
    pub(super) launched: u64,
}

pub(super) const fn failure_kind(error: SessionError) -> BackendFailureKind {
    match error {
        // The machine could not be built from artifacts the host presented as prepared, which
        // is a property of the host rather than of the request.
        SessionError::Create | SessionError::LaunchPage => BackendFailureKind::Unavailable,
        // The guest exists but never reached, or lost, its authenticated session.
        SessionError::Boot
        | SessionError::Ready
        | SessionError::Execute
        | SessionError::Gone
        | SessionError::Poisoned => BackendFailureKind::GuestFailure,
    }
}

impl KvmBackend {
    /// Serves this Launch from a machine claimed out of the pool.
    ///
    /// The claimed machine is either assigned here or destroyed: `assign` consumes the claim,
    /// and a failure never returns the machine to the pool.
    pub(super) fn assign_claimed(
        &mut self,
        operation: &OperationId,
        claimed: claim::ClaimedMachine,
        prepared: &PreparedGeneration,
        instance: &InstanceId,
    ) -> Result<Started, BackendFailure> {
        let assignment = claim::assignment_for(&claimed.snapshot, prepared, instance)
            .map_err(|kind| self.fail(operation, kind))?;
        // The machine already exists, so this stamp separates the fresh authority this Launch
        // built from the session it then drives, exactly as the on-demand arm does.
        let launched = self.clocks.elapsed_ns(operation);
        let session = claimed.machine.assign(assignment).map_err(|error| {
            BackendFailure::new(failure_kind(error), self.clocks.elapsed_ns(operation))
        })?;
        Ok(Started {
            preparation: PreparationClass::PreparedWorker,
            session,
            launched,
        })
    }

    /// Serves this Launch by building its own machine, because none was prepared for it.
    pub(super) fn restore_on_demand(
        &mut self,
        operation: &OperationId,
        prepared: &PreparedGeneration,
        instance: &InstanceId,
        memory_mib: u64,
    ) -> Result<Started, BackendFailure> {
        let boot =
            boot_for(prepared, memory_mib, instance).map_err(|kind| self.fail(operation, kind))?;
        let launched = self.clocks.elapsed_ns(operation);
        let session = Session::launch(boot).map_err(|error| {
            BackendFailure::new(failure_kind(error), self.clocks.elapsed_ns(operation))
        })?;
        Ok(Started {
            preparation: PreparationClass::OnDemand,
            session,
            launched,
        })
    }
}
