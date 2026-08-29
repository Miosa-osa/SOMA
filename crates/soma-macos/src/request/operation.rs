use std::fmt;

use serde::Serialize;

use super::{
    ControlLimits, ExecutionLimits, GuestCommand, ImageReference, InstanceId, MachineShape,
    NetworkConfiguration,
};

#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct OneShotRun {
    instance_id: InstanceId,
    image: ImageReference,
    shape: MachineShape,
    command: GuestCommand,
    limits: ExecutionLimits,
    network: NetworkConfiguration,
}

impl OneShotRun {
    #[must_use]
    pub const fn new(
        instance_id: InstanceId,
        image: ImageReference,
        shape: MachineShape,
        command: GuestCommand,
        limits: ExecutionLimits,
    ) -> Self {
        Self {
            instance_id,
            image,
            shape,
            command,
            limits,
            network: NetworkConfiguration::runtime_default(),
        }
    }

    #[must_use]
    pub fn with_network(mut self, network: NetworkConfiguration) -> Self {
        self.network = network;
        self
    }

    #[must_use]
    pub fn with_network_policy(self, policy: super::NetworkPolicy) -> Self {
        self.with_network(NetworkConfiguration::for_attachment(policy))
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub const fn image(&self) -> &ImageReference {
        &self.image
    }

    #[must_use]
    pub const fn shape(&self) -> MachineShape {
        self.shape
    }

    #[must_use]
    pub const fn command(&self) -> &GuestCommand {
        &self.command
    }

    #[must_use]
    pub const fn limits(&self) -> ExecutionLimits {
        self.limits
    }

    #[must_use]
    pub const fn network(&self) -> &NetworkConfiguration {
        &self.network
    }
}

impl fmt::Debug for OneShotRun {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OneShotRun")
            .field("instance_id", &self.instance_id)
            .field("image", &self.image)
            .field("shape", &self.shape)
            .field("command", &self.command)
            .field("limits", &self.limits)
            .field("network", &self.network)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct CreateMachine {
    instance_id: InstanceId,
    image: ImageReference,
    shape: MachineShape,
    init_command: GuestCommand,
    limits: ControlLimits,
    network: NetworkConfiguration,
}

impl CreateMachine {
    #[must_use]
    pub const fn new(
        instance_id: InstanceId,
        image: ImageReference,
        shape: MachineShape,
        init_command: GuestCommand,
        limits: ControlLimits,
    ) -> Self {
        Self {
            instance_id,
            image,
            shape,
            init_command,
            limits,
            network: NetworkConfiguration::runtime_default(),
        }
    }

    #[must_use]
    pub fn with_network(mut self, network: NetworkConfiguration) -> Self {
        self.network = network;
        self
    }

    #[must_use]
    pub fn with_network_policy(self, policy: super::NetworkPolicy) -> Self {
        self.with_network(NetworkConfiguration::for_attachment(policy))
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub const fn image(&self) -> &ImageReference {
        &self.image
    }

    #[must_use]
    pub const fn shape(&self) -> MachineShape {
        self.shape
    }

    #[must_use]
    pub const fn init_command(&self) -> &GuestCommand {
        &self.init_command
    }

    #[must_use]
    pub const fn limits(&self) -> ControlLimits {
        self.limits
    }

    #[must_use]
    pub const fn network(&self) -> &NetworkConfiguration {
        &self.network
    }
}

impl fmt::Debug for CreateMachine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateMachine")
            .field("instance_id", &self.instance_id)
            .field("image", &self.image)
            .field("shape", &self.shape)
            .field("init_command", &self.init_command)
            .field("limits", &self.limits)
            .field("network", &self.network)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct ExecuteCommand {
    instance_id: InstanceId,
    command: GuestCommand,
    limits: ExecutionLimits,
}

impl ExecuteCommand {
    #[must_use]
    pub const fn new(
        instance_id: InstanceId,
        command: GuestCommand,
        limits: ExecutionLimits,
    ) -> Self {
        Self {
            instance_id,
            command,
            limits,
        }
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub const fn command(&self) -> &GuestCommand {
        &self.command
    }

    #[must_use]
    pub const fn limits(&self) -> ExecutionLimits {
        self.limits
    }
}

impl fmt::Debug for ExecuteCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecuteCommand")
            .field("instance_id", &self.instance_id)
            .field("command", &self.command)
            .field("limits", &self.limits)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct StopOptions {
    grace_seconds: u32,
    limits: ControlLimits,
}

impl StopOptions {
    #[must_use]
    pub const fn new(grace_seconds: u32, limits: ControlLimits) -> Self {
        Self {
            grace_seconds,
            limits,
        }
    }

    #[must_use]
    pub const fn grace_seconds(self) -> u32 {
        self.grace_seconds
    }

    #[must_use]
    pub const fn limits(self) -> ControlLimits {
        self.limits
    }
}
