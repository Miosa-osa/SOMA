//! Bounded typed sections: role, version, length, digest, critical flag, payload.
//!
//! The section sequence is canonical: roles are strictly ascending, so duplicates and
//! reordering are rejected by the same rule.
//! Unknown critical roles reject the whole snapshot; unknown non-critical roles are skipped
//! after their digest is verified.

use std::{error::Error, fmt};

use super::{
    Digest, WireError,
    wire::{Reader, Writer},
};

/// Version of every known section encoding in schema v1.
pub const SECTION_VERSION: u16 = 1;
/// Upper bound on one section payload.
pub const MAX_SECTION_BYTES: u32 = 1 << 20;
/// Upper bound on the number of sections in one manifest.
pub const MAX_SECTIONS: u16 = 32;
/// Encoded size of one section header.
pub const HEADER_LEN: usize = 2 + 2 + 1 + 4 + Digest::LEN;

const FLAG_CRITICAL: u8 = 0b0000_0001;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SectionRole {
    VmState,
    Vcpu0,
    Irqchip,
    IrqRouting,
    KvmClock,
    Pit,
    Device0,
    Device1,
    Device2,
    Device3,
    Device4,
    RepairPointMarker,
}

impl SectionRole {
    pub const ALL: [Self; 12] = [
        Self::VmState,
        Self::Vcpu0,
        Self::Irqchip,
        Self::IrqRouting,
        Self::KvmClock,
        Self::Pit,
        Self::Device0,
        Self::Device1,
        Self::Device2,
        Self::Device3,
        Self::Device4,
        Self::RepairPointMarker,
    ];

    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::VmState => 0x0001,
            Self::Vcpu0 => 0x0002,
            Self::Irqchip => 0x0003,
            Self::IrqRouting => 0x0004,
            Self::KvmClock => 0x0005,
            Self::Pit => 0x0006,
            Self::Device0 => 0x0010,
            Self::Device1 => 0x0011,
            Self::Device2 => 0x0012,
            Self::Device3 => 0x0013,
            Self::Device4 => 0x0014,
            Self::RepairPointMarker => 0x0020,
        }
    }

    #[must_use]
    pub fn from_code(code: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.code() == code)
    }

    /// Whether the codec refuses a manifest that lacks this section.
    ///
    /// The two optional device slots are structurally optional here because a Generation that
    /// declared neither writable storage nor a network never had those devices to capture. That
    /// is not a weaker check: which slots a manifest must carry is a statement about a
    /// particular Generation rather than about the format, so it is the compatibility check,
    /// which knows the device set, that requires exactly the sections that machine has and
    /// refuses any other combination.
    #[must_use]
    pub const fn is_required(self) -> bool {
        !matches!(self, Self::Pit | Self::Device1 | Self::Device2)
    }

    /// The device slot carried by a device section.
    #[must_use]
    pub const fn device_slot(self) -> Option<u8> {
        match self {
            Self::Device0 => Some(0),
            Self::Device1 => Some(1),
            Self::Device2 => Some(2),
            Self::Device3 => Some(3),
            Self::Device4 => Some(4),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SectionError {
    Wire(WireError),
    PayloadTooLarge { length: u64 },
    UnknownCriticalRole(u16),
    UnsupportedVersion { role: u16, version: u16 },
    ReservedFlags(u8),
    KnownRoleNotCritical(SectionRole),
    DigestMismatch { role: u16 },
    RoleOrder { previous: u16, next: u16 },
}

impl fmt::Display for SectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(error) => write!(formatter, "section wire error: {error}"),
            Self::PayloadTooLarge { length } => {
                write!(formatter, "section payload of {length} bytes exceeds bound")
            }
            Self::UnknownCriticalRole(code) => {
                write!(formatter, "unknown critical section role {code:#06x}")
            }
            Self::UnsupportedVersion { role, version } => {
                write!(
                    formatter,
                    "section role {role:#06x} version {version} unsupported"
                )
            }
            Self::ReservedFlags(flags) => write!(formatter, "reserved section flags {flags:#04x}"),
            Self::KnownRoleNotCritical(role) => {
                write!(formatter, "known section {role:?} must be critical")
            }
            Self::DigestMismatch { role } => {
                write!(formatter, "section role {role:#06x} digest mismatch")
            }
            Self::RoleOrder { previous, next } => write!(
                formatter,
                "section role {next:#06x} must follow {previous:#06x} in ascending order"
            ),
        }
    }
}

impl Error for SectionError {}

impl From<WireError> for SectionError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

/// One known, bounded, digest-covered section.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Section {
    role: SectionRole,
    payload: Vec<u8>,
}

impl Section {
    /// Wraps a payload whose length is within [`MAX_SECTION_BYTES`].
    ///
    /// # Errors
    ///
    /// Returns [`SectionError::PayloadTooLarge`] when the payload exceeds the bound.
    pub fn new(role: SectionRole, payload: Vec<u8>) -> Result<Self, SectionError> {
        let length = u64::try_from(payload.len()).unwrap_or(u64::MAX);
        if length > u64::from(MAX_SECTION_BYTES) {
            return Err(SectionError::PayloadTooLarge { length });
        }
        Ok(Self { role, payload })
    }

    #[must_use]
    pub const fn role(&self) -> SectionRole {
        self.role
    }

    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    #[must_use]
    pub fn digest(&self) -> Digest {
        Digest::of(&self.payload)
    }

    pub(crate) fn encode(&self, writer: &mut Writer) {
        writer.put_u16(self.role.code());
        writer.put_u16(SECTION_VERSION);
        writer.put_u8(FLAG_CRITICAL);
        // `new` bounded the length to `MAX_SECTION_BYTES`, which fits in `u32`.
        writer.put_u32(u32::try_from(self.payload.len()).unwrap_or(MAX_SECTION_BYTES));
        writer.put_bytes(self.digest().as_bytes());
        writer.put_bytes(&self.payload);
    }

    /// Decodes `count` sections in strictly ascending role order.
    ///
    /// # Errors
    ///
    /// Returns a typed [`SectionError`] for any malformed, duplicated, reordered, unknown
    /// critical, unsupported, or digest-mismatched section.
    pub(crate) fn decode_sequence(
        reader: &mut Reader<'_>,
        count: u16,
    ) -> Result<Vec<Self>, SectionError> {
        let mut sections = Vec::with_capacity(usize::from(count.min(MAX_SECTIONS)));
        let mut previous: Option<u16> = None;
        for _ in 0..count {
            let code = reader.u16()?;
            if let Some(previous) = previous
                && code <= previous
            {
                return Err(SectionError::RoleOrder {
                    previous,
                    next: code,
                });
            }
            previous = Some(code);
            if let Some(section) = Self::decode_body(reader, code)? {
                sections.push(section);
            }
        }
        Ok(sections)
    }

    fn decode_body(reader: &mut Reader<'_>, code: u16) -> Result<Option<Self>, SectionError> {
        let version = reader.u16()?;
        let flags = reader.u8()?;
        if flags & !FLAG_CRITICAL != 0 {
            return Err(SectionError::ReservedFlags(flags));
        }
        let critical = flags & FLAG_CRITICAL != 0;
        let role = SectionRole::from_code(code);
        match role {
            Some(_) if version != SECTION_VERSION => {
                return Err(SectionError::UnsupportedVersion {
                    role: code,
                    version,
                });
            }
            Some(role) if !critical => return Err(SectionError::KnownRoleNotCritical(role)),
            None if critical => return Err(SectionError::UnknownCriticalRole(code)),
            _ => {}
        }
        let length = reader.length_u32(MAX_SECTION_BYTES)?;
        let expected = Digest::from_bytes(reader.array()?);
        let payload = reader.take(length)?;
        if Digest::of(payload) != expected {
            return Err(SectionError::DigestMismatch { role: code });
        }
        Ok(role.map(|role| Self {
            role,
            payload: payload.to_vec(),
        }))
    }
}

#[cfg(test)]
mod tests;
