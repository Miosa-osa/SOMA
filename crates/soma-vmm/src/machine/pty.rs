//! One bounded terminal operation against the ready guest session.

use super::{
    Machine, State,
    fault::{instance_mismatch, lifecycle_failure},
};
use crate::{
    CleanupEvidence, Failure, FailureKind, FailurePhase, Milestones, Recovery, control::PtyRequest,
};

impl Machine {
    /// Performs one terminal exchange without changing lifecycle state.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when the machine is not ready, the Instance differs, or the
    /// authenticated guest session cannot answer with certainty.
    pub fn pty(&mut self, request: &PtyRequest) -> Result<soma::PtyAnswer, Failure> {
        if self.state != State::Ready {
            return Err(lifecycle_failure());
        }
        if self.instance_id != Some(request.instance_id()) {
            return Err(instance_mismatch());
        }
        self.platform.pty(request.operation()).map_err(|failure| {
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
