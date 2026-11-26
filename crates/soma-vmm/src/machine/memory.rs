use super::{
    Machine, State,
    fault::{instance_mismatch, lifecycle_failure},
};
use crate::sandbox::{MemoryError, MemoryLevel};
use crate::{CleanupEvidence, Failure, FailureKind, FailurePhase, InstanceId, Milestones};

impl Machine {
    /// Asks the matching Instance's memory device to plug `requested` bytes in total, or with
    /// `None` reads its level.
    ///
    /// The device's own refusal is an answer, not a failure: the machine keeps serving.
    ///
    /// # Errors
    ///
    /// Returns a typed failure if the identity or lifecycle does not match, or the session could
    /// not confirm the exchange, which ends the machine as an uncertain command does.
    pub fn memory(
        &mut self,
        instance: InstanceId,
        requested: Option<u64>,
    ) -> Result<Result<MemoryLevel, MemoryError>, Failure> {
        if self.state != State::Ready {
            return Err(lifecycle_failure());
        }
        if self.instance_id != Some(instance) {
            return Err(instance_mismatch());
        }
        self.platform.memory(requested).map_err(|failure| {
            self.state = State::Failed;
            self.guest = None;
            Failure::new(
                FailureKind::ExecuteFailed,
                FailurePhase::Execute,
                failure.recovery(),
                Milestones::default(),
                CleanupEvidence::NotRequired,
            )
        })
    }
}
