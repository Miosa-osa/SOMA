use std::ffi::OsString;

use crate::{
    BackendError, CleanupState, CommandFailure, CommandFailureReason, ControlLimits,
    ControlReceipt, CreateMachine, CreatedMachine, ExecuteCommand, ExecutionResult,
    ExecutionStatus, InspectedMachine, InstanceId, Operation, ProcessFailureKind, StopOptions,
};

use super::{MacOsBackend, network::append_network, one_shot::execution_result};

impl MacOsBackend {
    /// Creates a stopped OCI-backed virtual machine with a caller-selected init command.
    ///
    /// # Errors
    ///
    /// Returns a typed command or cleanup failure.
    pub fn create(&self, request: &CreateMachine) -> Result<CreatedMachine, BackendError> {
        self.ensure_host()?;
        let output = self.commands.execute_control(
            Operation::Create,
            create_arguments(request),
            request.limits(),
        );
        match output {
            Ok(output) if output.status().is_success() => Ok(CreatedMachine::new(
                request.instance_id().clone(),
                request.instance_id().container_name(),
            )),
            Ok(output) => {
                let primary = CommandFailure::new(
                    Operation::Create,
                    CommandFailureReason::Status(output.status()),
                );
                self.failed_create(request.instance_id(), primary)
            }
            Err(primary) => self.failed_create(request.instance_id(), primary),
        }
    }

    /// Starts a previously created machine.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the runtime cannot prove a successful start.
    pub fn start(
        &self,
        instance_id: InstanceId,
        limits: ControlLimits,
    ) -> Result<ControlReceipt, BackendError> {
        self.control(
            instance_id,
            Operation::Start,
            vec![OsString::from("start")],
            limits,
        )
    }

    /// Executes a bounded command inside a running machine.
    ///
    /// Timeout, output-limit, signal, and post-spawn process-control uncertainty terminally
    /// invalidate the machine because stopping the host CLI does not prove the guest command
    /// stopped.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the host-side command cannot be executed.
    pub fn execute(&self, request: &ExecuteCommand) -> Result<ExecutionResult, BackendError> {
        self.ensure_host()?;
        let inspection = self
            .inspect_owned(request.instance_id())
            .map_err(BackendError::command)?;
        let resources = inspection.resources;
        let mut arguments = vec![
            OsString::from("exec"),
            OsString::from(request.instance_id().container_name()),
            OsString::from(request.command().program()),
        ];
        arguments.extend(request.command().arguments().iter().map(OsString::from));
        match self
            .commands
            .execute_guest(Operation::Execute, arguments, request.limits())
        {
            Ok(output) if execution_requires_invalidation(output.status()) => {
                match self.force_delete_owned(request.instance_id()) {
                    Ok(cleanup) => Ok(execution_result(
                        request.instance_id().clone(),
                        output,
                        Some(CleanupState::Complete),
                        cleanup.resources.or(resources),
                        cleanup.published_ports,
                    )),
                    Err(cleanup) => Err(BackendError::CleanupFailed {
                        instance_id: request.instance_id().clone(),
                        primary_failed: true,
                        cleanup,
                    }),
                }
            }
            Ok(output) => Ok(execution_result(
                request.instance_id().clone(),
                output,
                None,
                resources,
                None,
            )),
            Err(failure) if failure_requires_invalidation(failure.reason()) => {
                match self.force_delete_owned(request.instance_id()) {
                    Ok(cleanup) => Err(BackendError::ManagedExecutionInvalidated {
                        instance_id: request.instance_id().clone(),
                        failure,
                        cleanup: CleanupState::Complete,
                        cleanup_published_ports: cleanup.published_ports,
                    }),
                    Err(cleanup) => Err(BackendError::CleanupFailed {
                        instance_id: request.instance_id().clone(),
                        primary_failed: true,
                        cleanup,
                    }),
                }
            }
            Err(failure) => Err(BackendError::command(failure)),
        }
    }

    /// Gracefully stops a running machine using the requested grace period.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the runtime cannot prove a successful stop.
    pub fn stop(
        &self,
        instance_id: InstanceId,
        options: StopOptions,
    ) -> Result<ControlReceipt, BackendError> {
        self.control(
            instance_id,
            Operation::Stop,
            vec![
                OsString::from("stop"),
                OsString::from("--time"),
                OsString::from(options.grace_seconds().to_string()),
            ],
            options.limits(),
        )
    }

    /// Force-deletes a machine and all runtime state owned by its deterministic name.
    ///
    /// # Errors
    ///
    /// Returns a typed error when absence cannot be proven by a successful delete.
    pub fn delete(
        &self,
        instance_id: InstanceId,
        limits: ControlLimits,
    ) -> Result<ControlReceipt, BackendError> {
        self.control(
            instance_id,
            Operation::Delete,
            vec![OsString::from("delete"), OsString::from("--force")],
            limits,
        )
    }

    /// Returns the runtime's bounded JSON inspection document.
    ///
    /// # Errors
    ///
    /// Returns a typed error for command failure or invalid JSON.
    pub fn inspect(
        &self,
        instance_id: InstanceId,
        _limits: ControlLimits,
    ) -> Result<InspectedMachine, BackendError> {
        self.ensure_host()?;
        let inspection = self
            .inspect_owned(&instance_id)
            .map_err(BackendError::command)?;
        Ok(InspectedMachine::new(
            instance_id,
            inspection.document,
            inspection.resources,
            inspection.network,
        ))
    }

    fn control(
        &self,
        instance_id: InstanceId,
        operation: Operation,
        mut arguments: Vec<OsString>,
        limits: ControlLimits,
    ) -> Result<ControlReceipt, BackendError> {
        self.ensure_host()?;
        self.inspect_owned(&instance_id)
            .map_err(BackendError::command)?;
        arguments.push(OsString::from(instance_id.container_name()));
        let output = self
            .commands
            .execute_control(operation, arguments, limits)
            .map_err(BackendError::command)?;
        require_success(operation, output.status())?;
        Ok(ControlReceipt::new(instance_id, operation))
    }

    fn failed_create(
        &self,
        instance_id: &InstanceId,
        primary: CommandFailure,
    ) -> Result<CreatedMachine, BackendError> {
        match self.force_delete_owned(instance_id) {
            Ok(_) => Err(BackendError::command(primary)),
            Err(cleanup) => Err(BackendError::CleanupFailed {
                instance_id: instance_id.clone(),
                primary_failed: true,
                cleanup,
            }),
        }
    }
}

const fn execution_requires_invalidation(status: ExecutionStatus) -> bool {
    matches!(
        status,
        ExecutionStatus::Signaled
            | ExecutionStatus::TimedOut
            | ExecutionStatus::OutputLimitExceeded
    )
}

const fn failure_requires_invalidation(reason: CommandFailureReason) -> bool {
    match reason {
        CommandFailureReason::Process(
            ProcessFailureKind::PipeUnavailable
            | ProcessFailureKind::ReadFailed
            | ProcessFailureKind::WaitFailed
            | ProcessFailureKind::KillFailed
            | ProcessFailureKind::ReaderPanicked,
        ) => true,
        CommandFailureReason::Status(status) => execution_requires_invalidation(status),
        CommandFailureReason::Process(
            ProcessFailureKind::ExecutableUnavailable
            | ProcessFailureKind::PermissionDenied
            | ProcessFailureKind::SpawnFailed,
        )
        | CommandFailureReason::Ownership(_)
        | CommandFailureReason::InvalidJson
        | CommandFailureReason::MissingVersionComponent
        | CommandFailureReason::RuntimeNotRunning => false,
    }
}

fn create_arguments(request: &CreateMachine) -> Vec<OsString> {
    let shape = request.shape();
    let mut arguments = vec![
        OsString::from("create"),
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
        OsString::from("--entrypoint"),
        OsString::from(request.init_command().program()),
        OsString::from(request.image().as_str()),
    ]);
    arguments.extend(
        request
            .init_command()
            .arguments()
            .iter()
            .map(OsString::from),
    );
    arguments
}

fn require_success(
    operation: Operation,
    status: crate::ExecutionStatus,
) -> Result<(), BackendError> {
    if status.is_success() {
        Ok(())
    } else {
        Err(BackendError::command(CommandFailure::new(
            operation,
            CommandFailureReason::Status(status),
        )))
    }
}
