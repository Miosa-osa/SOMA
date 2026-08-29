use crate::{
    CommandStatus, DirectCommand, ExecutionLimits, InstanceId, ObservedOutput, OperationId,
};

#[derive(Clone, Copy)]
pub struct ExecutionRequest<'a> {
    operation_id: &'a OperationId,
    instance_id: &'a InstanceId,
    command: &'a DirectCommand,
    limits: &'a ExecutionLimits,
}

impl<'a> ExecutionRequest<'a> {
    pub(crate) const fn new(
        operation_id: &'a OperationId,
        instance_id: &'a InstanceId,
        command: &'a DirectCommand,
        limits: &'a ExecutionLimits,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            command,
            limits,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        self.instance_id
    }

    #[must_use]
    pub const fn command(&self) -> &DirectCommand {
        self.command
    }

    #[must_use]
    pub const fn limits(&self) -> &ExecutionLimits {
        self.limits
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandTimes {
    started_at_ns: u64,
    finished_at_ns: u64,
}

impl CommandTimes {
    #[must_use]
    pub const fn new(started_at_ns: u64, finished_at_ns: u64) -> Self {
        Self {
            started_at_ns,
            finished_at_ns,
        }
    }

    pub(crate) const fn values(self) -> [u64; 2] {
        [self.started_at_ns, self.finished_at_ns]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandObservation {
    operation_id: OperationId,
    instance_id: InstanceId,
    status: CommandStatus,
    output: ObservedOutput,
    times: CommandTimes,
}

impl CommandObservation {
    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        status: CommandStatus,
        output: ObservedOutput,
        times: CommandTimes,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            status,
            output,
            times,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        OperationId,
        InstanceId,
        CommandStatus,
        ObservedOutput,
        CommandTimes,
    ) {
        (
            self.operation_id,
            self.instance_id,
            self.status,
            self.output,
            self.times,
        )
    }
}
