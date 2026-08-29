use soma::{
    BackendFailure, BackendFailureKind, CommandObservation, ExecutionRequest, ObservedOutput,
};
use soma_macos::{ExecuteCommand, ExecutionLimits, GuestCommand};

use super::{
    adapter::MacBackend,
    config::mac_instance,
    evidence::command_status,
    failure::{error_proved_invalidation, invalidation_cleanup_ports},
    network::{effective_publications, verify_released},
};

impl MacBackend {
    pub(in crate::backend) fn execute(
        &mut self,
        request: ExecutionRequest<'_>,
    ) -> Result<CommandObservation, BackendFailure> {
        let operation = request.operation_id();
        let started = self.clocks.elapsed_ns(operation);
        let instance = mac_instance(request.instance_id().as_str())
            .map_err(|_| self.failure(operation, BackendFailureKind::WorkloadRejected))?;
        let command = GuestCommand::new(
            request.command().executable(),
            request.command().arguments().iter().map(String::as_str),
        )
        .map_err(|_| self.failure(operation, BackendFailureKind::WorkloadRejected))?;
        let limits = ExecutionLimits::new(
            request.limits().timeout_ms(),
            request.limits().max_output_bytes(),
        )
        .map_err(|_| self.failure(operation, BackendFailureKind::WorkloadRejected))?;
        let execution = match self
            .backend
            .execute(&ExecuteCommand::new(instance, command, limits))
        {
            Ok(execution) => execution,
            Err(error) => {
                if error_proved_invalidation(&error) {
                    let cleanup_is_verified = invalidation_cleanup_ports(&error)
                        .and_then(|ports| effective_publications(ports).ok())
                        .is_some_and(|ports| verify_released(&ports).is_ok());
                    if !cleanup_is_verified {
                        return Err(self.failure(operation, BackendFailureKind::CleanupFailure));
                    }
                    self.already_cleaned
                        .insert(request.instance_id().as_str().to_owned());
                }
                return Err(self.map_error(operation, &error));
            }
        };
        if execution.cleanup().is_some() {
            let cleanup_is_verified = execution
                .cleanup_published_ports()
                .and_then(|ports| effective_publications(ports).ok())
                .is_some_and(|ports| verify_released(&ports).is_ok());
            if !cleanup_is_verified {
                return Err(self.failure(operation, BackendFailureKind::CleanupFailure));
            }
            self.already_cleaned
                .insert(request.instance_id().as_str().to_owned());
        }
        let status = command_status(execution.status());
        let output = ObservedOutput::new(
            execution.stdout().to_vec(),
            execution.stdout_observed_bytes(),
            execution.stderr().to_vec(),
            execution.stderr_observed_bytes(),
        );
        Ok(CommandObservation::new(
            operation.clone(),
            request.instance_id().clone(),
            status,
            output,
            soma::CommandTimes::new(started, self.clocks.elapsed_ns(operation)),
        ))
    }
}
