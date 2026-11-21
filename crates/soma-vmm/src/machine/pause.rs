use super::{
    Machine, State,
    fault::{instance_mismatch, lifecycle_failure},
};
use crate::{CleanupEvidence, Failure, FailureKind, FailurePhase, InstanceId, Milestones};

impl Machine {
    /// Pauses the matching Instance without releasing it.
    ///
    /// # Errors
    ///
    /// Returns a typed failure if the identity or lifecycle does not match, or the platform
    /// cannot confirm that the guest paused.
    pub fn pause(&mut self, instance: InstanceId) -> Result<(), Failure> {
        if !matches!(self.state, State::Ready | State::Paused) {
            return Err(lifecycle_failure());
        }
        if self.instance_id != Some(instance) {
            return Err(instance_mismatch());
        }
        if self.state == State::Paused {
            return Ok(());
        }
        self.platform.pause().map_err(|failure| {
            self.state = State::Failed;
            self.guest = None;
            Failure::new(
                FailureKind::PauseFailed,
                FailurePhase::Pause,
                failure.recovery(),
                Milestones::default(),
                CleanupEvidence::NotRequired,
            )
        })?;
        self.state = State::Paused;
        Ok(())
    }

    /// Resumes the matching resident Instance without replacing its authenticated session.
    ///
    /// # Errors
    ///
    /// Returns a typed failure if the Instance or its lifecycle does not match, or the
    /// platform cannot confirm that the guest resumed.
    pub fn resume(&mut self, instance: InstanceId) -> Result<(), Failure> {
        if !matches!(self.state, State::Ready | State::Paused) {
            return Err(lifecycle_failure());
        }
        if self.instance_id != Some(instance) {
            return Err(instance_mismatch());
        }
        if self.state == State::Ready {
            return Ok(());
        }
        self.platform.resume().map_err(|failure| {
            self.state = State::Failed;
            self.guest = None;
            Failure::new(
                FailureKind::ResumeFailed,
                FailurePhase::Resume,
                failure.recovery(),
                Milestones::default(),
                CleanupEvidence::NotRequired,
            )
        })?;
        self.state = State::Ready;
        Ok(())
    }
}
