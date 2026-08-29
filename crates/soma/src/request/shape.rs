use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use super::{NetworkPolicy, ValidationError};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    network: NetworkPolicy,
}

impl Capabilities {
    /// Requires the backend to deny network access.
    #[must_use]
    pub const fn isolated() -> Self {
        Self {
            network: NetworkPolicy::isolated(),
        }
    }

    /// Leaves network policy to the backend while requiring it to report what happened.
    #[must_use]
    pub const fn unspecified() -> Self {
        Self {
            network: NetworkPolicy::runtime_default(),
        }
    }

    /// Requires the backend to provide network access.
    ///
    /// # Panics
    ///
    /// Panics only if SOMA's built-in public-network policy violates its own invariant gate.
    #[must_use]
    pub fn with_network_access(mut self) -> Self {
        self.network = NetworkPolicy::new(
            super::EgressPolicy::PublicInternet,
            super::DnsPolicy::System,
            Vec::new(),
        )
        .expect("the built-in connected network policy is valid");
        self
    }

    /// Requests the backend's unfiltered development network.
    ///
    /// # Panics
    ///
    /// Panics only if SOMA's built-in unrestricted policy violates its own invariant gate.
    #[must_use]
    pub fn with_unrestricted_network(mut self) -> Self {
        self.network = NetworkPolicy::new(
            super::EgressPolicy::Unrestricted,
            super::DnsPolicy::System,
            Vec::new(),
        )
        .expect("the built-in unrestricted network policy is valid");
        self
    }

    #[must_use]
    pub fn with_network_policy(mut self, network: NetworkPolicy) -> Self {
        self.network = network;
        self
    }

    #[must_use]
    pub const fn network_policy(&self) -> &NetworkPolicy {
        &self.network
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MachineShape {
    vcpu_count: u16,
    memory_mib: u64,
    storage_mib: u64,
    capabilities: Capabilities,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MachineShapeWire {
    vcpu_count: u16,
    memory_mib: u64,
    storage_mib: u64,
    capabilities: Capabilities,
}

impl<'de> Deserialize<'de> for MachineShape {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = MachineShapeWire::deserialize(deserializer)?;
        Self::new(wire.vcpu_count, wire.memory_mib, wire.storage_mib)
            .map(|shape| shape.with_capabilities(wire.capabilities))
            .map_err(D::Error::custom)
    }
}

impl MachineShape {
    pub const MIN_VCPU_COUNT: u16 = 1;
    pub const MAX_VCPU_COUNT: u16 = u16::MAX;
    pub const MIN_MEMORY_MIB: u64 = 1;
    pub const MAX_MEMORY_MIB: u64 = u64::MAX;
    pub const MIN_STORAGE_MIB: u64 = 1;
    pub const MAX_STORAGE_MIB: u64 = u64::MAX;
    pub const DEFAULT_VCPU_COUNT: u16 = 1;
    pub const DEFAULT_MEMORY_MIB: u64 = 1_024;
    pub const DEFAULT_STORAGE_MIB: u64 = 10_240;

    /// Creates a provider-neutral requested shape.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidShape`] when any dimension is zero.
    pub fn new(
        vcpu_count: u16,
        memory_mib: u64,
        storage_mib: u64,
    ) -> Result<Self, ValidationError> {
        if !(Self::MIN_VCPU_COUNT..=Self::MAX_VCPU_COUNT).contains(&vcpu_count)
            || !(Self::MIN_MEMORY_MIB..=Self::MAX_MEMORY_MIB).contains(&memory_mib)
            || !(Self::MIN_STORAGE_MIB..=Self::MAX_STORAGE_MIB).contains(&storage_mib)
        {
            return Err(ValidationError::InvalidShape);
        }
        Ok(Self {
            vcpu_count,
            memory_mib,
            storage_mib,
            capabilities: Capabilities::isolated(),
        })
    }

    #[must_use]
    pub fn with_capabilities(mut self, capabilities: Capabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    #[must_use]
    pub fn vcpu_count(&self) -> u16 {
        self.vcpu_count
    }

    #[must_use]
    pub fn memory_mib(&self) -> u64 {
        self.memory_mib
    }

    #[must_use]
    pub fn storage_mib(&self) -> u64 {
        self.storage_mib
    }

    #[must_use]
    pub const fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }
}
