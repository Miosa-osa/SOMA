use std::time::Duration;

use soma::{CommandObservation, CommandStatus, CommandTimes, ExecutionRequest};

use super::DockerBackend;
use super::command::command_owned;
use super::container::{container_name, remove};
use super::process;

impl DockerBackend {
    pub(in crate::backend) fn execute(
        &mut self,
        request: ExecutionRequest<'_>,
    ) -> CommandObservation {
        let operation = request.operation_id();
        let started = self.clocks.elapsed_ns(operation);
        let name = container_name(request.instance_id().as_str());
        let mut args = vec![
            "exec".to_owned(),
            name.clone(),
            request.command().executable().to_owned(),
        ];
        args.extend(request.command().arguments().iter().map(ToOwned::to_owned));
        let result = command_owned(&args, Duration::from_millis(request.limits().timeout_ms()));
        // A container is recorded as already cleaned only when its removal was proven. Marking
        // it regardless made the later Cleanup report a complete release for a container that
        // may still be running, which is cleanup evidence the host cannot back.
        if (result.timed_out || result.output_limited) && remove(&name) {
            self.already_cleaned
                .insert(request.instance_id().as_str().to_owned());
        }
        let status = if result.timed_out {
            CommandStatus::TimedOut
        } else if result.output_limited {
            CommandStatus::OutputLimitExceeded
        } else {
            CommandStatus::Exited {
                code: process::status_code(result.status),
            }
        };
        CommandObservation::new(
            operation.clone(),
            request.instance_id().clone(),
            status,
            soma::ObservedOutput::new(
                result.stdout.clone(),
                result.stdout.len() as u64,
                result.stderr.clone(),
                result.stderr.len() as u64,
            ),
            CommandTimes::new(started, self.clocks.elapsed_ns(operation)),
        )
    }
}
