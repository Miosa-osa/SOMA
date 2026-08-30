//! `state.somasnap` schema v2: fixed-order header, host-profile requirements, memory
//! descriptor, and the bounded typed section sequence.
//!
//! Layout (big-endian, no padding):
//!
//! ```text
//! magic "SOMASNP\0"          8
//! schema version             u16
//! architecture               u16
//! page size                  u32
//! candidate id               32
//! machine contract digest    32
//! device contract digest     32
//! cpu template digest        32
//! host requirements          u32 api, u16 count, count x u16 capability, u16 min slots
//! memory descriptor          32 digest, u64 size
//! vcpu count                 u16
//! guest protocol version     u16
//! section count              u16
//! sections                   count x (u16 role, u16 version, u8 flags, u32 len, 32 digest, payload)
//! ```

mod header;
mod host;
#[cfg(test)]
pub(crate) mod tests;

use std::{error::Error, fmt};

pub use header::{Architecture, CandidateId, ManifestHeader, PageSize};
pub use host::{
    HostCapability, HostRequirements, HostRequirementsError, MAX_REQUIRED_CAPABILITIES,
};

use super::{
    WireError,
    memory::MemoryError,
    section::{MAX_SECTIONS, Section, SectionError, SectionRole},
    wire::{Reader, Writer},
};

pub const MAGIC: [u8; 8] = *b"SOMASNP\0";
pub const SCHEMA_VERSION: u16 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestError {
    Wire(WireError),
    BadMagic,
    UnsupportedSchemaVersion(u16),
    UnknownArchitecture(u16),
    InvalidPageSize(u32),
    ZeroCandidateId,
    HostRequirements(HostRequirementsError),
    Memory(MemoryError),
    ZeroVcpuCount,
    TooManySections(u16),
    Section(SectionError),
    MissingRequiredSection(SectionRole),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(error) => write!(formatter, "manifest wire error: {error}"),
            Self::BadMagic => formatter.write_str("manifest magic is not SOMASNP"),
            Self::UnsupportedSchemaVersion(version) => {
                write!(formatter, "unsupported snapshot schema version {version}")
            }
            Self::UnknownArchitecture(code) => {
                write!(formatter, "unknown architecture code {code}")
            }
            Self::InvalidPageSize(size) => write!(formatter, "invalid page size {size}"),
            Self::ZeroCandidateId => formatter.write_str("candidate id cannot be all zero"),
            Self::HostRequirements(error) => write!(formatter, "{error}"),
            Self::Memory(error) => write!(formatter, "{error}"),
            Self::ZeroVcpuCount => formatter.write_str("vCPU count must be non-zero"),
            Self::TooManySections(count) => {
                write!(
                    formatter,
                    "{count} sections exceed the bound {MAX_SECTIONS}"
                )
            }
            Self::Section(error) => write!(formatter, "{error}"),
            Self::MissingRequiredSection(role) => {
                write!(formatter, "required section {role:?} is absent")
            }
        }
    }
}

impl Error for ManifestError {}

impl From<WireError> for ManifestError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

impl From<SectionError> for ManifestError {
    fn from(error: SectionError) -> Self {
        Self::Section(error)
    }
}

impl From<MemoryError> for ManifestError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

impl From<HostRequirementsError> for ManifestError {
    fn from(error: HostRequirementsError) -> Self {
        Self::HostRequirements(error)
    }
}

/// One validated snapshot state manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Manifest {
    header: ManifestHeader,
    sections: Vec<Section>,
}

impl Manifest {
    /// Builds a manifest from a header and sections in strictly ascending role order.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::TooManySections`], [`ManifestError::Section`] for a
    /// duplicate or reordered role, or [`ManifestError::MissingRequiredSection`].
    pub fn new(header: ManifestHeader, sections: Vec<Section>) -> Result<Self, ManifestError> {
        if sections.len() > usize::from(MAX_SECTIONS) {
            return Err(ManifestError::TooManySections(
                u16::try_from(sections.len()).unwrap_or(u16::MAX),
            ));
        }
        for pair in sections.windows(2) {
            let (previous, next) = (pair[0].role().code(), pair[1].role().code());
            if next <= previous {
                return Err(SectionError::RoleOrder { previous, next }.into());
            }
        }
        for role in SectionRole::ALL {
            if role.is_required() && !sections.iter().any(|section| section.role() == role) {
                return Err(ManifestError::MissingRequiredSection(role));
            }
        }
        Ok(Self { header, sections })
    }

    #[must_use]
    pub const fn header(&self) -> &ManifestHeader {
        &self.header
    }

    #[must_use]
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    #[must_use]
    pub fn section(&self, role: SectionRole) -> Option<&Section> {
        self.sections.iter().find(|section| section.role() == role)
    }

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::with_capacity(4096);
        writer.put_bytes(&MAGIC);
        writer.put_u16(SCHEMA_VERSION);
        self.header.encode(&mut writer);
        // `new` bounded the count to `MAX_SECTIONS`.
        writer.put_u16(u16::try_from(self.sections.len()).unwrap_or(MAX_SECTIONS));
        for section in &self.sections {
            section.encode(&mut writer);
        }
        writer.finish()
    }

    /// Decodes and fully validates hostile manifest bytes.
    ///
    /// # Errors
    ///
    /// Returns a typed [`ManifestError`] for every malformed, unsupported, duplicated,
    /// reordered, absent, oversized, digest-mismatched, or trailing input.
    pub fn decode(bytes: &[u8]) -> Result<Self, ManifestError> {
        let mut reader = Reader::new(bytes);
        if reader.array::<8>()? != MAGIC {
            return Err(ManifestError::BadMagic);
        }
        let schema = reader.u16()?;
        if schema != SCHEMA_VERSION {
            return Err(ManifestError::UnsupportedSchemaVersion(schema));
        }
        let header = ManifestHeader::decode(&mut reader)?;
        let count = reader.u16()?;
        if count > MAX_SECTIONS {
            return Err(ManifestError::TooManySections(count));
        }
        let sections = Section::decode_sequence(&mut reader, count)?;
        reader.finish()?;
        Self::new(header, sections)
    }
}
