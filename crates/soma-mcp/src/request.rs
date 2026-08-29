use crate::{InstanceId, OperationId};

pub use soma::{
    DirectCommand, ExecutionLimits, MachineName as DisplayName, MachineShape, OciImage,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendTarget {
    #[default]
    Auto,
    Local,
    Kvm,
    Macos,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MachineDefinition {
    image: OciImage,
    display_name: Option<DisplayName>,
    shape: MachineShape,
    backend: BackendTarget,
}

impl MachineDefinition {
    pub(crate) const fn new(
        image: OciImage,
        display_name: Option<DisplayName>,
        shape: MachineShape,
        backend: BackendTarget,
    ) -> Self {
        Self {
            image,
            display_name,
            shape,
            backend,
        }
    }

    fn into_parts(self) -> (OciImage, Option<DisplayName>, MachineShape, BackendTarget) {
        (self.image, self.display_name, self.shape, self.backend)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunRequest {
    operation_id: OperationId,
    instance_id: InstanceId,
    machine: MachineDefinition,
    command: DirectCommand,
    limits: ExecutionLimits,
}

impl RunRequest {
    pub(crate) const fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        machine: MachineDefinition,
        command: DirectCommand,
        limits: ExecutionLimits,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            machine,
            command,
            limits,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub const fn image(&self) -> &OciImage {
        &self.machine.image
    }

    #[must_use]
    pub const fn display_name(&self) -> Option<&DisplayName> {
        self.machine.display_name.as_ref()
    }

    #[must_use]
    pub const fn command(&self) -> &DirectCommand {
        &self.command
    }

    #[must_use]
    pub const fn shape(&self) -> &MachineShape {
        &self.machine.shape
    }

    #[must_use]
    pub const fn limits(&self) -> &ExecutionLimits {
        &self.limits
    }

    #[must_use]
    pub const fn backend(&self) -> BackendTarget {
        self.machine.backend
    }

    pub(crate) fn into_facade(self) -> soma::RunRequest {
        let (image, name, shape, _) = self.machine.into_parts();
        let mut request = soma::RunRequest::new(
            self.operation_id,
            self.instance_id,
            image,
            shape,
            self.command,
            self.limits,
        );
        if let Some(name) = name {
            request = request.with_name(name);
        }
        request
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchRequest {
    operation_id: OperationId,
    instance_id: InstanceId,
    machine: MachineDefinition,
}

impl LaunchRequest {
    pub(crate) const fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        machine: MachineDefinition,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            machine,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub const fn image(&self) -> &OciImage {
        &self.machine.image
    }

    #[must_use]
    pub const fn display_name(&self) -> Option<&DisplayName> {
        self.machine.display_name.as_ref()
    }

    #[must_use]
    pub const fn shape(&self) -> &MachineShape {
        &self.machine.shape
    }

    #[must_use]
    pub const fn backend(&self) -> BackendTarget {
        self.machine.backend
    }

    pub(crate) fn into_facade(self) -> soma::LaunchMachineRequest {
        let (image, name, shape, _) = self.machine.into_parts();
        let mut request =
            soma::LaunchMachineRequest::new(self.operation_id, self.instance_id, image, shape);
        if let Some(name) = name {
            request = request.with_name(name);
        }
        request
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecRequest {
    operation_id: OperationId,
    instance_id: InstanceId,
    command: DirectCommand,
    limits: ExecutionLimits,
    backend: BackendTarget,
}

impl ExecRequest {
    pub(crate) const fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        command: DirectCommand,
        limits: ExecutionLimits,
        backend: BackendTarget,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            command,
            limits,
            backend,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub const fn command(&self) -> &DirectCommand {
        &self.command
    }

    #[must_use]
    pub const fn limits(&self) -> &ExecutionLimits {
        &self.limits
    }

    #[must_use]
    pub const fn backend(&self) -> BackendTarget {
        self.backend
    }

    pub(crate) fn into_facade(self) -> soma::ExecuteMachineRequest {
        soma::ExecuteMachineRequest::new(
            self.operation_id,
            self.instance_id,
            self.command,
            self.limits,
        )
    }
}
