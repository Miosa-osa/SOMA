use super::{
    Machine, State,
    fault::{instance_mismatch, lifecycle_failure, operation_conflict},
};
use crate::{
    CleanupEvidence, Failure, FailureKind, FailurePhase, Milestone, Milestones, Stop, Stopped,
    operation::StopReplay,
};

impl Machine {
    /// Stops the matching Instance and proves cleanup before returning success.
    ///
    /// # Errors
    ///
    /// Returns a typed [`Failure`] when lifecycle or identity does not match, the operation
    /// conflicts, or cleanup is incomplete.
    pub fn stop(&mut self, request: Stop) -> Result<Stopped, Failure> {
        match self.operations.replay_stop(&request) {
            Ok(StopReplay::Complete(outcome)) => return outcome,
            Ok(StopReplay::Continue) => {}
            Err(_) => return Err(operation_conflict()),
            Ok(StopReplay::New) => {
                self.validate_stop(&request)?;
                self.operations.admit_stop(request.clone());
            }
        }

        let outcome = self.perform_stop(request);
        if outcome.is_ok() {
            self.operations.complete_stop(outcome.clone());
        }
        outcome
    }

    fn perform_stop(&mut self, request: Stop) -> Result<Stopped, Failure> {
        self.validate_stop(&request)?;

        let mut milestones = Milestones::default();
        milestones.push(Milestone::StopRequested);
        let evidence = self
            .platform
            .stop(&request, self.guest.as_mut())
            .map_err(|failure| {
                self.state = State::Failed;
                Failure::new(
                    FailureKind::StopFailed,
                    FailurePhase::Stop,
                    failure.recovery(),
                    milestones.clone(),
                    CleanupEvidence::Incomplete,
                )
            })?;

        milestones.push(Milestone::CleanupCompleted);
        milestones.push(Milestone::Stopped);
        self.state = State::Stopped;
        self.guest = None;
        let (operation_id, instance_id) = request.into_ids();
        Ok(Stopped::new(
            operation_id,
            instance_id,
            evidence.guest_acknowledged(),
            evidence.forced(),
            CleanupEvidence::Complete,
            milestones,
        ))
    }

    fn validate_stop(&self, request: &Stop) -> Result<(), Failure> {
        if !matches!(self.state, State::Ready | State::Failed | State::Launching) {
            return Err(lifecycle_failure());
        }
        if self.instance_id != Some(request.instance_id()) {
            return Err(instance_mismatch());
        }
        Ok(())
    }
}
