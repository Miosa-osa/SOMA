//! Every way a section can be malformed, and how each one reads.

use std::{error::Error, fmt};

use super::SectionRole;
use crate::snapshot::WireError;

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
