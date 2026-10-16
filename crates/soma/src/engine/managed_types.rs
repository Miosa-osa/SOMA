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
    pub(super) volume: Option<VolumeAttachment>,
}

/// The durable volume one Instance's machine binds, as the host resolved it.
///
/// The id is what the ready signal reports back to the control plane; the path is the host's
/// own, derived by the volume store from the volume this Instance owns. A caller never names a
/// file, and a launch that carries no attachment is a launch whose Generation declares no
/// volume: a machine that declares one refuses to run without it rather than serve an empty
/// disk where the data belongs.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VolumeAttachment {
    /// The volume identity, as the launch receipt reports it.
    pub id: [u8; 16],
    /// The host's path to the volume file.
    pub path: std::path::PathBuf,
}

impl VolumeAttachment {
    /// One attachment, as the volume store answered for this Instance.
    #[must_use]
    pub fn new(id: [u8; 16], path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            id,
            path: path.into(),
        }
    }
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
            volume: None,
        }
    }

    /// This request with the durable volume the Instance's machine must bind.
    #[must_use]
    pub fn with_volume(mut self, volume: VolumeAttachment) -> Self {
        self.volume = Some(volume);
        self
    }

    /// The durable volume this launch binds, when it binds one.
    #[must_use]
    pub const fn volume(&self) -> Option<&VolumeAttachment> {
        self.volume.as_ref()
    }

    #[must_use]
    pub fn with_name(mut self, machine_name: MachineName) -> Self {
        self.machine_name = Some(machine_name);
        self
    }

    /// The optional metadata name the launch carries.
    #[must_use]
    pub const fn machine_name(&self) -> Option<&MachineName> {
        self.machine_name.as_ref()
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

    /// The command this request asks the instance to run.
    #[must_use]
    pub const fn command(&self) -> &DirectCommand {
        &self.command
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
    /// The operation this stop belongs to, which is the idempotency key a stop also carries to
    /// the volume transactions it drives.
    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

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
