//! Host-profile requirement block: KVM API version, required capabilities, memory slots.

use std::{error::Error, fmt};

use super::ManifestError;
use crate::snapshot::wire::{Reader, Writer};

pub const MAX_REQUIRED_CAPABILITIES: u16 = 32;

/// KVM capabilities a host must report; codes are stable wire values.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum HostCapability {
    UserMemory,
    IrqChip,
    IrqFd,
    IoEventFd,
    ImmediateExit,
    Xsave,
    Xcrs,
    VcpuEvents,
    MpState,
    NestedState,
    AdjustClock,
    SetTssAddr,
    Pit2,
}

impl HostCapability {
    pub const ALL: [Self; 13] = [
        Self::UserMemory,
        Self::IrqChip,
        Self::IrqFd,
        Self::IoEventFd,
        Self::ImmediateExit,
        Self::Xsave,
        Self::Xcrs,
        Self::VcpuEvents,
        Self::MpState,
        Self::NestedState,
        Self::AdjustClock,
        Self::SetTssAddr,
        Self::Pit2,
    ];

    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::UserMemory => 1,
            Self::IrqChip => 2,
            Self::IrqFd => 3,
            Self::IoEventFd => 4,
            Self::ImmediateExit => 5,
            Self::Xsave => 6,
            Self::Xcrs => 7,
            Self::VcpuEvents => 8,
            Self::MpState => 9,
            Self::NestedState => 10,
            Self::AdjustClock => 11,
            Self::SetTssAddr => 12,
            Self::Pit2 => 13,
        }
    }

    #[must_use]
    pub fn from_code(code: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|cap| cap.code() == code)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostRequirementsError {
    TooManyCapabilities(u16),
    UnknownCapability(u16),
    CapabilityOrder { previous: u16, next: u16 },
    ZeroMemorySlots,
}

impl fmt::Display for HostRequirementsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyCapabilities(count) => {
                write!(formatter, "{count} required capabilities exceed the bound")
            }
            Self::UnknownCapability(code) => {
                write!(formatter, "unknown required capability code {code}")
            }
            Self::CapabilityOrder { previous, next } => write!(
                formatter,
                "capability {next} must follow {previous} in ascending order"
            ),
            Self::ZeroMemorySlots => formatter.write_str("minimum memory slots must be non-zero"),
        }
    }
}

impl Error for HostRequirementsError {}

/// Host-profile requirements a restoring host must satisfy exactly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostRequirements {
    kvm_api_version: u32,
    capabilities: Vec<HostCapability>,
    min_memory_slots: u16,
}

impl HostRequirements {
    /// Creates requirements from a strictly ascending, bounded capability list.
    ///
    /// # Errors
    ///
    /// Returns a [`HostRequirementsError`] for an oversized or unordered list or zero slots.
    pub fn new(
        kvm_api_version: u32,
        capabilities: Vec<HostCapability>,
        min_memory_slots: u16,
    ) -> Result<Self, HostRequirementsError> {
        if capabilities.len() > usize::from(MAX_REQUIRED_CAPABILITIES) {
            return Err(HostRequirementsError::TooManyCapabilities(
                u16::try_from(capabilities.len()).unwrap_or(u16::MAX),
            ));
        }
        for pair in capabilities.windows(2) {
            let (previous, next) = (pair[0].code(), pair[1].code());
            if next <= previous {
                return Err(HostRequirementsError::CapabilityOrder { previous, next });
            }
        }
        if min_memory_slots == 0 {
            return Err(HostRequirementsError::ZeroMemorySlots);
        }
        Ok(Self {
            kvm_api_version,
            capabilities,
            min_memory_slots,
        })
    }

    #[must_use]
    pub const fn kvm_api_version(&self) -> u32 {
        self.kvm_api_version
    }

    #[must_use]
    pub fn capabilities(&self) -> &[HostCapability] {
        &self.capabilities
    }

    #[must_use]
    pub const fn min_memory_slots(&self) -> u16 {
        self.min_memory_slots
    }

    pub(super) fn encode(&self, writer: &mut Writer) {
        writer.put_u32(self.kvm_api_version);
        writer.put_u16(u16::try_from(self.capabilities.len()).unwrap_or(u16::MAX));
        for capability in &self.capabilities {
            writer.put_u16(capability.code());
        }
        writer.put_u16(self.min_memory_slots);
    }

    pub(super) fn decode(reader: &mut Reader<'_>) -> Result<Self, ManifestError> {
        let kvm_api_version = reader.u32()?;
        let count = reader.count_u16(MAX_REQUIRED_CAPABILITIES)?;
        let mut capabilities = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            let code = reader.u16()?;
            capabilities.push(
                HostCapability::from_code(code)
                    .ok_or(HostRequirementsError::UnknownCapability(code))?,
            );
        }
        let min_memory_slots = reader.u16()?;
        Ok(Self::new(kvm_api_version, capabilities, min_memory_slots)?)
    }
}
