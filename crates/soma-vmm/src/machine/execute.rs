use super::{
    Machine, State,
    fault::{
        instance_mismatch, lifecycle_failure, operation_capacity_exceeded, operation_conflict,
    },
};
use crate::{
    CleanupEvidence, Execute, Executed, ExitStatus, Failure, FailureKind, FailurePhase, Milestones,
    platform::PlatformExecution,
};

impl Machine {
    /// Executes one bounded command through the repaired authenticated guest channel.
    ///
    /// # Errors
    ///
    /// Returns a typed [`Failure`] when the Instance is not Ready, identity does not match, the
    /// operation conflicts, or authenticated execution fails.
    pub fn execute(&mut self, request: Execute) -> Result<Executed, Failure> {
        match self.operations.replay_execute(&request) {
            Ok(Some(outcome)) => return outcome,
            Err(_) => return Err(operation_conflict()),
            Ok(None) => {}
        }
        self.validate_execute(&request)?;
        self.operations
            .ensure_execute_capacity(&request)
            .map_err(|_| operation_capacity_exceeded())?;

        let outcome = self.perform_execute(&request);
        self.operations.record_execute(request, outcome.clone());
        outcome
    }

    fn perform_execute(&mut self, request: &Execute) -> Result<Executed, Failure> {
        let guest = self.guest.as_mut().ok_or_else(lifecycle_failure)?;
        let outcome = match self.platform.execute(request, guest) {
            Ok(outcome) => outcome,
            Err(failure) => {
                self.state = State::Failed;
                self.guest = None;
                return Err(Failure::new(
                    FailureKind::ExecuteFailed,
                    FailurePhase::Execute,
                    failure.recovery(),
                    Milestones::default(),
                    CleanupEvidence::NotRequired,
                ));
            }
        };

        let (status, stdout, stderr) = bounded_output(outcome, request);
        Ok(Executed::new(
            request.operation_id(),
            request.instance_id(),
            status,
            stdout,
            stderr,
        ))
    }

    fn validate_execute(&self, request: &Execute) -> Result<(), Failure> {
        if self.state != State::Ready {
            return Err(lifecycle_failure());
        }
        if self.instance_id != Some(request.instance_id()) {
            return Err(instance_mismatch());
        }
        Ok(())
    }
}

fn bounded_output(outcome: PlatformExecution, request: &Execute) -> (ExitStatus, Vec<u8>, Vec<u8>) {
    let maximum = usize::try_from(request.limits().output().get()).unwrap_or(usize::MAX);
    let (mut status, mut stdout, mut stderr) = outcome.into_parts();
    if stdout.len().saturating_add(stderr.len()) > maximum {
        stdout.truncate(maximum);
        stderr.truncate(maximum.saturating_sub(stdout.len()));
        status = ExitStatus::OutputLimit;
    }
    (status, stdout, stderr)
}
