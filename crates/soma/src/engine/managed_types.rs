use crate::{
    CapturedOutput, DirectCommand, ExecutionLimits, ExecutionReceipt, InstanceId, MachineName,
    MachineState, OciImage, OperationId,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchMachineRequest {
    pub(super) operation_id: OperationId,
    pub(super) instance_id: InstanceId,
    pub(super) image: OciImage,
    pub(super) shape: crate::MachineShape,
    pub(super) machine_name: Option<MachineName>,
}

impl LaunchMachineRequest {
    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        image: OciImage,
        shape: crate::MachineShape,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            image,
            shape,
            machine_name: None,
        }
    }

    #[must_use]
    pub fn with_name(mut self, machine_name: MachineName) -> Self {
        self.machine_name = Some(machine_name);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecuteMachineRequest {
    pub(super) operation_id: OperationId,
    pub(super) instance_id: InstanceId,
    pub(super) command: DirectCommand,
    pub(super) limits: ExecutionLimits,
}

impl ExecuteMachineRequest {
    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        command: DirectCommand,
        limits: ExecutionLimits,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            command,
            limits,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopMachineRequest {
    pub(super) operation_id: OperationId,
    pub(super) instance_id: InstanceId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectMachineRequest {
    pub(super) operation_id: OperationId,
    pub(super) instance_id: InstanceId,
}

impl InspectMachineRequest {
    #[must_use]
    pub const fn new(operation_id: OperationId, instance_id: InstanceId) -> Self {
        Self {
            operation_id,
            instance_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DestroyMachineRequest {
    pub(super) operation_id: OperationId,
    pub(super) instance_id: InstanceId,
}

impl DestroyMachineRequest {
    #[must_use]
    pub const fn new(operation_id: OperationId, instance_id: InstanceId) -> Self {
        Self {
            operation_id,
            instance_id,
        }
    }
}

impl StopMachineRequest {
    #[must_use]
    pub const fn new(operation_id: OperationId, instance_id: InstanceId) -> Self {
        Self {
            operation_id,
            instance_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineLaunch {
    pub(super) receipt: ExecutionReceipt,
}

impl MachineLaunch {
    #[must_use]
    pub const fn receipt(&self) -> &ExecutionReceipt {
        &self.receipt
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineExecution {
    pub(super) receipt: ExecutionReceipt,
    pub(super) output: CapturedOutput,
}

impl MachineExecution {
    #[must_use]
    pub const fn receipt(&self) -> &ExecutionReceipt {
        &self.receipt
    }

    #[must_use]
    pub const fn output(&self) -> &CapturedOutput {
        &self.output
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineStop {
    pub(super) receipt: ExecutionReceipt,
}

impl MachineStop {
    #[must_use]
    pub const fn receipt(&self) -> &ExecutionReceipt {
        &self.receipt
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineInspection {
    pub(super) state: MachineState,
    pub(super) receipt: ExecutionReceipt,
}

impl MachineInspection {
    #[must_use]
    pub const fn state(&self) -> MachineState {
        self.state
    }

    #[must_use]
    pub const fn receipt(&self) -> &ExecutionReceipt {
        &self.receipt
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineDestroy {
    pub(super) receipt: ExecutionReceipt,
}

impl MachineDestroy {
    #[must_use]
    pub const fn receipt(&self) -> &ExecutionReceipt {
        &self.receipt
    }
}
