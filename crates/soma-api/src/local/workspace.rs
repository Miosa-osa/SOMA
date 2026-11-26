use soma::{InspectMachineRequest, InstanceId, ManagedFailure, ManagedStateError, OperationId};

use super::LocalFacade;
use crate::volumes::{VolumeBody, VolumeFailure, VolumeOperation, VolumeReport};

impl LocalFacade {
    /// Routes a live workspace change to the process that serializes operations on the VM.
    ///
    /// A stopped sandbox uses the existing offline volume operation.
    pub(super) fn live_workspace(
        &mut self,
        operation: VolumeOperation,
        body: &VolumeBody,
    ) -> Result<Option<VolumeReport>, VolumeFailure> {
        let operation_id =
            OperationId::new(body.operation_id.clone()).map_err(|_| VolumeFailure::Malformed)?;
        let instance =
            InstanceId::new(body.instance_id.clone()).map_err(|_| VolumeFailure::Malformed)?;
        let state = self
            .runtime
            .inspect_machine(InspectMachineRequest::new(operation_id, instance.clone()))
            .map(|inspection| inspection.state());
        if running(&state)? {
            self.change_while_running(operation, body, &instance)
                .map(Some)
        } else {
            Ok(None)
        }
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn change_while_running(
        &self,
        operation: VolumeOperation,
        body: &VolumeBody,
        instance: &InstanceId,
    ) -> Result<VolumeReport, VolumeFailure> {
        use crate::volumes::{report_for, request_for};
        let request = request_for(operation, body)?;
        let reply = self
            .runtime
            .change_workspace(instance, &request)
            .map_err(refusal)?;
        report_for(operation, body, &reply)
    }

    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    #[allow(
        clippy::unused_self,
        reason = "the Linux implementation uses the facade's runtime"
    )]
    fn change_while_running(
        &self,
        _operation: VolumeOperation,
        _body: &VolumeBody,
        _instance: &InstanceId,
    ) -> Result<VolumeReport, VolumeFailure> {
        Err(VolumeFailure::RequiresStopped)
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn refusal(kind: soma::BackendFailureKind) -> VolumeFailure {
    use soma::BackendFailureKind;
    use soma_hostd::{FailureCode, failure_code};

    match kind {
        BackendFailureKind::Unsupported | BackendFailureKind::SandboxPaused => {
            VolumeFailure::RequiresStopped
        }
        BackendFailureKind::ResourceConflict | BackendFailureKind::WorkloadRejected => {
            VolumeFailure::Refused(failure_code(FailureCode::VolumeBusy))
        }
        _ => VolumeFailure::Transport,
    }
}

/// Whether a volume change goes through the running machine, from what reading it answered.
///
/// A sandbox that is stopped, gone, or whose machine is lost reads as stopped or absent: there
/// is no machine to route the change through, so it takes the offline operation. Reading a
/// stopped sandbox as running sent every stop's detach to a machine that was not there, and the
/// stop answered `503 volume_transport`.
fn running(state: &Result<soma::MachineState, ManagedFailure>) -> Result<bool, VolumeFailure> {
    match state {
        Ok(soma::MachineState::Stopped)
        | Err(ManagedFailure::State(
            ManagedStateError::MachineNotFound | ManagedStateError::MachineStopped,
        )) => Ok(false),
        Ok(_) => Ok(true),
        Err(ManagedFailure::State(_)) => Err(VolumeFailure::RequiresStopped),
        Err(_) => Err(VolumeFailure::Transport),
    }
}

#[cfg(test)]
mod tests {
    use soma::{MachineState, ManagedFailure, ManagedStateError};

    use super::running;

    #[test]
    fn a_stopped_or_absent_sandbox_takes_the_offline_volume_operation() {
        assert_eq!(running(&Ok(MachineState::Stopped)), Ok(false));
        assert_eq!(
            running(&Err(ManagedFailure::State(
                ManagedStateError::MachineNotFound
            ))),
            Ok(false)
        );
        assert_eq!(
            running(&Err(ManagedFailure::State(
                ManagedStateError::MachineStopped
            ))),
            Ok(false)
        );
    }

    #[test]
    fn a_running_sandbox_takes_the_live_operation() {
        assert_eq!(running(&Ok(MachineState::Ready)), Ok(true));
    }
}
