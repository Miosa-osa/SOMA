use serde::Serialize;
use serde_json::Value;

use crate::{InstanceId, Operation};

use super::{InspectedNetwork, NetworkAttachment};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct MachineResources {
    vcpus: u16,
    memory_bytes: u64,
}

impl MachineResources {
    pub(crate) const fn new(vcpus: u16, memory_bytes: u64) -> Self {
        Self {
            vcpus,
            memory_bytes,
        }
    }

    #[must_use]
    pub const fn vcpus(self) -> u16 {
        self.vcpus
    }

    #[must_use]
    pub const fn memory_bytes(self) -> u64 {
        self.memory_bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CreatedMachine {
    instance_id: InstanceId,
    container_name: String,
}

impl CreatedMachine {
    pub(crate) fn new(instance_id: InstanceId, container_name: String) -> Self {
        Self {
            instance_id,
            container_name,
        }
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub fn container_name(&self) -> &str {
        &self.container_name
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ControlReceipt {
    instance_id: InstanceId,
    operation: Operation,
}

impl ControlReceipt {
    pub(crate) const fn new(instance_id: InstanceId, operation: Operation) -> Self {
        Self {
            instance_id,
            operation,
        }
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub const fn operation(&self) -> Operation {
        self.operation
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InspectedMachine {
    instance_id: InstanceId,
    document: Value,
    resources: Option<MachineResources>,
    network: InspectedNetwork,
}

impl InspectedMachine {
    pub(crate) const fn new(
        instance_id: InstanceId,
        document: Value,
        resources: Option<MachineResources>,
        network: InspectedNetwork,
    ) -> Self {
        Self {
            instance_id,
            document,
            resources,
            network,
        }
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub const fn resources(&self) -> Option<MachineResources> {
        self.resources
    }

    #[must_use]
    pub const fn network(&self) -> &InspectedNetwork {
        &self.network
    }

    #[must_use]
    pub const fn network_attachment(&self) -> Option<NetworkAttachment> {
        self.network.attachment()
    }

    #[must_use]
    pub const fn document(&self) -> &Value {
        &self.document
    }
}
