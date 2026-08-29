use soma::{
    BackendFailure, BackendFailureKind, BackendKind, EgressPolicy, InspectionObservation,
    InspectionRequest, MachineState,
};

use super::command::{CONTROL_TIMEOUT, command};
use super::container::container_name;
use super::network::effective_network;
use super::{DockerBackend, failure};

impl DockerBackend {
    pub(in crate::backend) fn inspect(
        &mut self,
        request: InspectionRequest<'_>,
    ) -> Result<InspectionObservation, BackendFailure> {
        let name = container_name(request.instance_id().as_str());
        let result = command(
            &["inspect", "--format", "{{.State.Status}}", &name],
            CONTROL_TIMEOUT,
        );
        let state = match String::from_utf8_lossy(&result.stdout).trim() {
            "running" => MachineState::Ready,
            "created" | "stopped" | "exited" => MachineState::Stopping,
            _ => {
                return Err(failure(
                    request.operation_id(),
                    BackendFailureKind::GuestFailure,
                ));
            }
        };
        let mode = if request.shape().capabilities().network_policy().egress()
            == EgressPolicy::Unrestricted
        {
            "bridge"
        } else {
            "none"
        };
        Ok(InspectionObservation::observed(
            request,
            BackendKind::DockerContainer,
            state,
            effective_network(mode),
            self.clocks.elapsed_ns(request.operation_id()),
        ))
    }
}
