use super::{
    Machine, State,
    fault::{instance_mismatch, lifecycle_failure},
};
use crate::{CleanupEvidence, Failure, FailureKind, FailurePhase, InstanceId, Milestones};

impl Machine {
    /// Performs one filesystem operation against this ready, authenticated Instance.
    ///
    /// # Errors
    /// Returns a typed failure if the identity or lifecycle state does not match,
    /// or if the guest exchange could not be confirmed.
    pub fn file(
        &mut self,
        instance: InstanceId,
        operation: soma::FileOperation,
    ) -> Result<soma::FileAnswer, Failure> {
        if self.state != State::Ready {
            return Err(lifecycle_failure());
        }
        if self.instance_id != Some(instance) {
            return Err(instance_mismatch());
        }
        self.platform.file(operation).map_err(|failure| {
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
