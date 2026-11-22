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
        let inspect = self
            .runtime
            .inspect_machine(InspectMachineRequest::new(operation_id, instance.clone()));
        match inspect {
            Err(ManagedFailure::State(
                ManagedStateError::MachineNotFound | ManagedStateError::MachineStopped,
            )) => Ok(None),
            Ok(_) => self
                .change_while_running(operation, body, &instance)
                .map(Some),
            Err(ManagedFailure::State(_)) => Err(VolumeFailure::RequiresStopped),
            Err(_) => Err(VolumeFailure::Transport),
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
