use std::ffi::OsString;

use crate::{
    BackendError, CleanupState, ExecutionResult, ExecutionStatus, OneShotRun, Operation,
    process::ProcessOutput,
};

use super::{MacOsBackend, network::append_network};

impl MacOsBackend {
    /// Runs one bounded command from an OCI image and proves force-deletion before returning.
    ///
    /// A nonzero guest exit remains an [`ExecutionResult`] with an explicit status.
    /// Process failures are errors, and cleanup failures take precedence because absence could not
    /// be proven.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the host process cannot be controlled or cleanup cannot be
    /// proven.
    pub fn run(&self, request: &OneShotRun) -> Result<ExecutionResult, BackendError> {
        self.ensure_host()?;
        let instance_id = request.instance_id().clone();
        let primary =
            self.commands
                .execute_guest(Operation::Run, run_arguments(request), request.limits());
        let cleanup = self.force_delete_owned(request.instance_id());

        let cleanup = match cleanup {
            Ok(cleanup) => cleanup,
            Err(cleanup) => {
                return Err(BackendError::CleanupFailed {
                    instance_id,
                    primary_failed: primary
                        .as_ref()
                        .map_or(true, |output| !output.status().is_success()),
                    cleanup,
                });
            }
        };

        let output = primary.map_err(BackendError::command)?;
        Ok(execution_result(
            request.instance_id().clone(),
            output,
            Some(CleanupState::Complete),
            cleanup.resources,
            cleanup.published_ports,
        ))
    }
}

pub(super) fn execution_result(
    instance_id: crate::InstanceId,
    output: ProcessOutput,
    cleanup: Option<CleanupState>,
    resources: Option<crate::MachineResources>,
    cleanup_published_ports: Option<Vec<crate::PublishedPort>>,
) -> ExecutionResult {
    ExecutionResult::from_process(
        instance_id,
        output,
        cleanup,
        resources,
        cleanup_published_ports,
    )
}

fn run_arguments(request: &OneShotRun) -> Vec<OsString> {
    let shape = request.shape();
    let mut arguments = vec![
        OsString::from("run"),
        OsString::from("--name"),
        OsString::from(request.instance_id().container_name()),
        OsString::from("--label"),
        OsString::from(request.instance_id().ownership_label()),
        OsString::from("--cpus"),
        OsString::from(shape.vcpus().to_string()),
        OsString::from("--memory"),
        OsString::from(format!("{}M", shape.memory_mebibytes())),
    ];
    append_network(&mut arguments, request.network());
    arguments.extend([
        OsString::from("--progress"),
        OsString::from("none"),
        OsString::from("--entrypoint"),
        OsString::from(request.command().program()),
        OsString::from(request.image().as_str()),
    ]);
    arguments.extend(request.command().arguments().iter().map(OsString::from));
    arguments
}

const _: fn(ExecutionStatus) -> bool = ExecutionStatus::is_success;
