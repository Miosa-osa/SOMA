use std::{ffi::OsString, path::PathBuf, sync::Arc, time::Duration};

use crate::{
    CommandFailure, CommandFailureReason, ControlLimits, ExecutionLimits, Operation,
    ProcessFailureKind,
    process::{
        ProcessInvocation, ProcessOutput, ProcessRunner, SystemProcessRunner, constrain_output,
    },
};

pub(super) struct CommandExecutor {
    executable: PathBuf,
    runner: Arc<dyn ProcessRunner>,
}

impl CommandExecutor {
    pub(super) fn system(executable: PathBuf) -> Self {
        Self::new(executable, Arc::new(SystemProcessRunner))
    }

    pub(super) const fn new(executable: PathBuf, runner: Arc<dyn ProcessRunner>) -> Self {
        Self { executable, runner }
    }

    pub(super) fn execute(
        &self,
        operation: Operation,
        arguments: Vec<OsString>,
        timeout_millis: u64,
        output_bytes: u64,
    ) -> Result<ProcessOutput, CommandFailure> {
        let output_limit = usize::try_from(output_bytes).unwrap_or(usize::MAX);
        let invocation = ProcessInvocation::new(
            self.executable.clone(),
            arguments,
            Duration::from_millis(timeout_millis),
            output_limit,
        );
        let output = self
            .runner
            .run(&invocation)
            .map_err(|kind| process_command_failure(operation, kind))?;
        Ok(constrain_output(output, output_limit))
    }

    pub(super) fn execute_guest(
        &self,
        operation: Operation,
        arguments: Vec<OsString>,
        limits: ExecutionLimits,
    ) -> Result<ProcessOutput, CommandFailure> {
        self.execute(
            operation,
            arguments,
            limits.timeout_millis(),
            limits.output_bytes(),
        )
    }

    pub(super) fn execute_control(
        &self,
        operation: Operation,
        arguments: Vec<OsString>,
        limits: ControlLimits,
    ) -> Result<ProcessOutput, CommandFailure> {
        self.execute(
            operation,
            arguments,
            limits.timeout_millis(),
            limits.output_bytes(),
        )
    }
}

fn process_command_failure(operation: Operation, kind: ProcessFailureKind) -> CommandFailure {
    CommandFailure::new(operation, CommandFailureReason::Process(kind))
}
