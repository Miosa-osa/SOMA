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

nonzero_bytes!(MemoryBytes, "memory bytes");
nonzero_bytes!(DiskBytes, "writable disk bytes");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MachineSpec {
    vcpus: VcpuCount,
    memory: MemoryBytes,
    writable_disk: DiskBytes,
}

impl MachineSpec {
    #[must_use]
    pub const fn new(vcpus: VcpuCount, memory: MemoryBytes, writable_disk: DiskBytes) -> Self {
        Self {
            vcpus,
            memory,
            writable_disk,
        }
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

/// Immutable reference to one certified artifact set and its exact effective Machine dimensions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Generation {
    id: GenerationId,
    machine: MachineSpec,
}

impl Generation {
    #[must_use]
    pub const fn new(id: GenerationId, machine: MachineSpec) -> Self {
        Self { id, machine }
    }

    #[must_use]
    pub const fn id(&self) -> GenerationId {
        self.id
    }

    #[must_use]
    pub const fn machine(&self) -> MachineSpec {
        self.machine
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
