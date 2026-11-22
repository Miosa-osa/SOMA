use soma_guest::WorkspaceOutcome;

use super::{
    Machine, State,
    fault::{instance_mismatch, lifecycle_failure},
};
use crate::{Failure, InstanceId, sandbox::WorkspaceChange};

impl Machine {
    /// Changes the durable workspace of the matching running Instance.
    ///
    /// # Errors
    ///
    /// Refuses the wrong Instance or lifecycle and fails the machine on uncertainty.
    pub fn change_workspace(
        &mut self,
        instance: InstanceId,
        change: WorkspaceChange,
    ) -> Result<WorkspaceOutcome, Failure> {
        if self.instance_id != Some(instance) {
            return Err(instance_mismatch());
        }
        if self.state != State::Ready {
            return Err(lifecycle_failure());
        }
        self.platform.change_workspace(change).map_err(|_| {
            self.state = State::Failed;
            self.guest = None;
            lifecycle_failure()
        })
    }
}
