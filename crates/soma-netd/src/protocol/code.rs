//! Stable protocol codes for broker failures.

use crate::Error;

/// Maps one broker failure onto a stable protocol code.
#[must_use]
pub fn error_code(error: &Error) -> u16 {
    match error {
        Error::InvalidIntent(_) => 1,
        Error::InvalidProfile(_) => 2,
        Error::PoolExhausted => 3,
        Error::InvalidId(_) => 4,
        Error::MissingPrivilege(_) => 5,
        Error::Kernel { .. } => 6,
        Error::Tool { .. } => 7,
        Error::LedgerConflict => 8,
        Error::ReplayMismatch => 9,
        Error::LedgerCorrupt => 10,
        Error::NotAssigned => 11,
        Error::Drift(_) => 12,
        Error::Transfer(_) => 13,
        Error::PortUnavailable => 14,
        Error::Unimplemented(_) => 15,
        Error::InvalidState(_) => 16,
        Error::Protocol(_) => 17,
        Error::Unauthorized(_) => 18,
    }
}
