//! Constant-size manifest header: identity, contract digests, host requirements, memory
//! descriptor, and machine shape.

use std::fmt;

use super::{ManifestError, host::HostRequirements};
use crate::snapshot::{
    Digest,
    memory::MemoryDescriptor,
    wire::{Reader, Writer},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Architecture {
    X86_64,
}

impl Architecture {
    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::X86_64 => 1,
        }
    }

    #[must_use]
    pub const fn from_code(code: u16) -> Option<Self> {
        match code {
            1 => Some(Self::X86_64),
            _ => None,
        }
    }
}

/// Guest page size: a power of two between 4 KiB and 1 GiB.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageSize(u32);

impl PageSize {
    pub const FOUR_KIB: Self = Self(4096);

    /// # Errors
    ///
    /// Returns [`ManifestError::InvalidPageSize`] outside the supported power-of-two range.
    pub const fn new(value: u32) -> Result<Self, ManifestError> {
        if value.is_power_of_two() && value >= 4096 && value <= 1 << 30 {
            Ok(Self(value))
        } else {
            Err(ManifestError::InvalidPageSize(value))
        }
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Exact 32-byte Candidate identity captured before the ready Generation can be derived.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct CandidateId([u8; 32]);

impl CandidateId {
    /// # Errors
    ///
    /// Returns [`ManifestError::ZeroCandidateId`] when every byte is zero.
    pub fn new(bytes: [u8; 32]) -> Result<Self, ManifestError> {
        if bytes.iter().all(|byte| *byte == 0) {
            Err(ManifestError::ZeroCandidateId)
        } else {
            Ok(Self(bytes))
        }
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for CandidateId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "CandidateId({})", Digest::from_bytes(self.0))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestHeader {
    pub architecture: Architecture,
    pub page_size: PageSize,
    pub candidate_id: CandidateId,
    pub machine_contract: Digest,
    pub device_contract: Digest,
    pub cpu_template: Digest,
    pub host: HostRequirements,
    pub memory: MemoryDescriptor,
    pub vcpu_count: u16,
    pub guest_protocol_version: u16,
}

impl ManifestHeader {
    pub(super) fn encode(&self, writer: &mut Writer) {
        writer.put_u16(self.architecture.code());
        writer.put_u32(self.page_size.get());
        writer.put_bytes(self.candidate_id.as_bytes());
        writer.put_bytes(self.machine_contract.as_bytes());
        writer.put_bytes(self.device_contract.as_bytes());
        writer.put_bytes(self.cpu_template.as_bytes());
        self.host.encode(writer);
        self.memory.encode(writer);
        writer.put_u16(self.vcpu_count);
        writer.put_u16(self.guest_protocol_version);
    }

    pub(super) fn decode(reader: &mut Reader<'_>) -> Result<Self, ManifestError> {
        let architecture_code = reader.u16()?;
        let architecture = Architecture::from_code(architecture_code)
            .ok_or(ManifestError::UnknownArchitecture(architecture_code))?;
        let page_size = PageSize::new(reader.u32()?)?;
        let candidate_id = CandidateId::new(reader.array()?)?;
        let machine_contract = Digest::from_bytes(reader.array()?);
        let device_contract = Digest::from_bytes(reader.array()?);
        let cpu_template = Digest::from_bytes(reader.array()?);
        let host = HostRequirements::decode(reader)?;
        let memory = MemoryDescriptor::decode(reader, page_size.get())?;
        let vcpu_count = reader.u16()?;
        if vcpu_count == 0 {
            return Err(ManifestError::ZeroVcpuCount);
        }
        let guest_protocol_version = reader.u16()?;
        Ok(Self {
            architecture,
            page_size,
            candidate_id,
            machine_contract,
            device_contract,
            cpu_template,
            host,
            memory,
            vcpu_count,
            guest_protocol_version,
        })
    }
}
