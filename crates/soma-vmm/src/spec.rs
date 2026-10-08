use std::{
    error::Error,
    fmt,
    num::{NonZeroU16, NonZeroU64},
};

use crate::GenerationId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VcpuCount(NonZeroU16);

impl VcpuCount {
    /// Creates a non-zero virtual CPU count.
    ///
    /// # Errors
    ///
    /// Returns [`SpecError::Zero`] when `value` is zero.
    pub fn new(value: u16) -> Result<Self, SpecError> {
        NonZeroU16::new(value)
            .map(Self)
            .ok_or(SpecError::Zero("vCPU count"))
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0.get()
    }
}

macro_rules! nonzero_bytes {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub struct $name(NonZeroU64);

        impl $name {
            /// Creates a non-zero byte quantity.
            ///
            /// # Errors
            ///
            /// Returns [`SpecError::Zero`] when `value` is zero.
            pub fn new(value: u64) -> Result<Self, SpecError> {
                NonZeroU64::new(value)
                    .map(Self)
                    .ok_or(SpecError::Zero($label))
            }

            #[must_use]
            pub const fn get(self) -> u64 {
                self.0.get()
            }
        }
    };
}

/// The version of the machine contract a Generation was built under.
///
/// The provider layer is contract-neutral and cannot name the machine contract type itself, so
/// the version travels as data and the `x86_64` provider resolves it before it restores anything.
/// A version it does not implement is refused there rather than read back out of the snapshot,
/// which would only agree with itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContractVersion(NonZeroU16);

impl ContractVersion {
    /// Creates a non-zero contract version.
    ///
    /// # Errors
    ///
    /// Returns [`SpecError::Zero`] when `value` is zero.
    pub fn new(value: u16) -> Result<Self, SpecError> {
        NonZeroU16::new(value)
            .map(Self)
            .ok_or(SpecError::Zero("machine contract version"))
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0.get()
    }
}

nonzero_bytes!(MemoryBytes, "memory bytes");
nonzero_bytes!(DiskBytes, "writable disk bytes");

/// The exact effective dimensions of one certified machine.
///
/// The machine contract travels with them as the portable version that names it. The provider
/// layer is contract-neutral and cannot name the machine contract type itself, so the version is
/// carried here as data and the `x86_64` provider resolves it before it restores anything. A
/// version this build does not implement is refused by that resolution rather than being read
/// back out of the snapshot, which would only agree with itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MachineSpec {
    vcpus: VcpuCount,
    memory: MemoryBytes,
    writable_disk: DiskBytes,
    contract: ContractVersion,
}

impl MachineSpec {
    /// Names the shape of one machine, at the contract version the version 1 machine is.
    ///
    /// The default is the oldest contract this build implements, so a caller that knows only the
    /// shape cannot accidentally claim a machine contract it never asserted. A caller that does
    /// know names it with [`Self::with_contract`].
    #[must_use]
    pub const fn new(vcpus: VcpuCount, memory: MemoryBytes, writable_disk: DiskBytes) -> Self {
        Self {
            vcpus,
            memory,
            writable_disk,
            contract: ContractVersion(NonZeroU16::MIN),
        }
    }

    /// Names the contract version this machine was built under.
    #[must_use]
    pub const fn with_contract(self, contract: ContractVersion) -> Self {
        Self { contract, ..self }
    }

    /// The contract version this machine was built under.
    #[must_use]
    pub const fn contract(self) -> ContractVersion {
        self.contract
    }

    #[must_use]
    pub const fn vcpus(self) -> VcpuCount {
        self.vcpus
    }

    #[must_use]
    pub const fn memory(self) -> MemoryBytes {
        self.memory
    }

    #[must_use]
    pub const fn writable_disk(self) -> DiskBytes {
        self.writable_disk
    }
}

/// The optional devices a Generation declared.
///
/// The machine a provider builds must be the machine the Generation was certified as, so the
/// declaration travels with the Generation rather than being read back out of the artifacts.
/// An artifact set can only agree with itself; the point of naming the set here is that the
/// machine the caller asked for and the machine the artifacts describe are checked against
/// each other.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeclaredDevices {
    writable_disk: bool,
    network: bool,
}

impl DeclaredDevices {
    #[must_use]
    pub const fn new(writable_disk: bool, network: bool) -> Self {
        Self {
            writable_disk,
            network,
        }
    }

    /// Whether this Generation declared writable storage, and so has a private overlay.
    #[must_use]
    pub const fn writable_disk(self) -> bool {
        self.writable_disk
    }

    /// Whether this Generation declared a network device.
    #[must_use]
    pub const fn network(self) -> bool {
        self.network
    }
}

/// Immutable reference to one certified artifact set and its exact effective Machine dimensions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Generation {
    id: GenerationId,
    machine: MachineSpec,
    devices: DeclaredDevices,
}

impl Generation {
    #[must_use]
    pub const fn new(id: GenerationId, machine: MachineSpec, devices: DeclaredDevices) -> Self {
        Self {
            id,
            machine,
            devices,
        }
    }

    #[must_use]
    pub const fn id(&self) -> GenerationId {
        self.id
    }

    #[must_use]
    pub const fn machine(&self) -> MachineSpec {
        self.machine
    }

    #[must_use]
    pub const fn devices(&self) -> DeclaredDevices {
        self.devices
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpecError {
    Zero(&'static str),
}

impl fmt::Display for SpecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zero(field) => write!(formatter, "{field} must be non-zero"),
        }
    }
}

impl Error for SpecError {}
