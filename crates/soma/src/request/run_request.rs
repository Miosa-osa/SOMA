use crate::{InstanceId, MachineName, OperationId};

use super::{DirectCommand, ExecutionLimits, MachineShape, OciImage};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunRequest {
    operation_id: OperationId,
    instance_id: InstanceId,
    image: OciImage,
    shape: MachineShape,
    command: DirectCommand,
    limits: ExecutionLimits,
    machine_name: Option<MachineName>,
}

impl RunRequest {
    #[must_use]
    pub fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        image: OciImage,
        shape: MachineShape,
        command: DirectCommand,
        limits: ExecutionLimits,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            image,
            shape,
            command,
            limits,
            machine_name: None,
        }
    }

    #[must_use]
    pub fn with_name(mut self, machine_name: MachineName) -> Self {
        self.machine_name = Some(machine_name);
        self
    }

    pub(crate) fn parts(
        &self,
    ) -> (
        &OperationId,
        &InstanceId,
        &OciImage,
        &MachineShape,
        &DirectCommand,
        &ExecutionLimits,
        Option<&MachineName>,
    ) {
        (
            &self.operation_id,
            &self.instance_id,
            &self.image,
            &self.shape,
            &self.command,
            &self.limits,
            self.machine_name.as_ref(),
        )
    }
}
