//! The bounded typed daemon protocol carried in `SOCK_SEQPACKET` frames.
//!
//! One request frame produces one reply frame; no frame carries a path, a descriptor number,
//! or free text, and every failure is a stable code.

mod reply;
mod request;

use std::fmt;

pub use reply::Reply;
pub use request::{LaunchFrame, Request};

use soma_netd::MAX_ENCODED_INTENT;

use crate::{ClaimError, InstanceError, LifecycleError, TransferFailure};

/// The largest request or reply frame.
pub const MAX_FRAME: usize = 1 + 16 + 16 + 4 + 8 + 32 + MAX_ENCODED_INTENT;

const CLAIM_HEADER: usize = 1 + 16 + 16 + 4 + 8 + 32;

/// A malformed frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtocolError(pub &'static str);

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "protocol error: {}", self.0)
    }
}

impl std::error::Error for ProtocolError {}

/// Stable failure codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum FailureCode {
    /// Malformed frame.
    Protocol = 1,
    /// Pool exhausted.
    Exhausted = 2,
    /// Bounded structure full.
    Overloaded = 3,
    /// Changed intent under a replayed operation, or a second operation presenting an
    /// Instance identity that is already owned.
    Conflict = 4,
    /// Claim deadline.
    Deadline = 5,
    /// Ledger failure.
    Ledger = 6,
    /// Construction failure.
    Construction = 7,
    /// Transfer failure.
    Transfer = 8,
    /// Unknown worker or Instance.
    Unknown = 9,
    /// Wrong phase.
    Phase = 10,
    /// Internal invariant.
    Invariant = 11,
    /// A capacity gate refused the Instance.
    Capacity = 12,
    /// The operation's worker was destroyed; its Launch did not succeed.
    Terminated = 13,
}

impl FailureCode {
    /// Returns the code one wire value names, or `None` when this build does not know it.
    ///
    /// An unknown code is answered with `None` rather than a catch-all variant so that a
    /// client serving an older vocabulary reports the number it actually received instead of
    /// collapsing a refusal it has never seen into one it has.
    #[must_use]
    pub const fn from_wire(value: u16) -> Option<Self> {
        match value {
            1 => Some(Self::Protocol),
            2 => Some(Self::Exhausted),
            3 => Some(Self::Overloaded),
            4 => Some(Self::Conflict),
            5 => Some(Self::Deadline),
            6 => Some(Self::Ledger),
            7 => Some(Self::Construction),
            8 => Some(Self::Transfer),
            9 => Some(Self::Unknown),
            10 => Some(Self::Phase),
            11 => Some(Self::Invariant),
            12 => Some(Self::Capacity),
            13 => Some(Self::Terminated),
            _ => None,
        }
    }
}

/// Maps a claim failure onto its code.
#[must_use]
pub const fn claim_failure_code(error: &ClaimError) -> FailureCode {
    match error {
        ClaimError::Exhausted(_) => FailureCode::Exhausted,
        ClaimError::Capacity(_) => FailureCode::Capacity,
        ClaimError::Overloaded(_) => FailureCode::Overloaded,
        ClaimError::OperationConflict { .. } => FailureCode::Conflict,
        ClaimError::Deadline { .. } => FailureCode::Deadline,
        ClaimError::Ledger(_) => FailureCode::Ledger,
        ClaimError::Construction(_) => FailureCode::Construction,
        ClaimError::MissingPayload(_) => FailureCode::Invariant,
    }
}

/// Maps a transfer failure onto its code.
#[must_use]
pub const fn transfer_failure_code(_failure: &TransferFailure) -> FailureCode {
    FailureCode::Transfer
}

/// Maps a lifecycle failure onto its code.
#[must_use]
pub const fn lifecycle_failure_code(error: &LifecycleError) -> FailureCode {
    match error {
        LifecycleError::Unknown(_) => FailureCode::Unknown,
        LifecycleError::Phase { .. } => FailureCode::Phase,
        LifecycleError::Start(_) => FailureCode::Transfer,
        LifecycleError::Ledger(_) => FailureCode::Ledger,
        LifecycleError::State(_) => FailureCode::Invariant,
    }
}

/// Maps an Instance lifecycle refusal onto its code.
///
/// An Instance refusal reuses the allocator's codes wherever it means the same thing, so a
/// client learns one vocabulary: a claim, transfer, or lifecycle refusal keeps the code the
/// worker operation would have returned.
#[must_use]
pub const fn instance_failure_code(error: &InstanceError) -> FailureCode {
    match error {
        InstanceError::Claim(error) => claim_failure_code(error),
        InstanceError::Transfer(failure) => transfer_failure_code(failure),
        InstanceError::Occupied { .. } => FailureCode::Conflict,
        InstanceError::Terminated(_) => FailureCode::Terminated,
        InstanceError::Unknown(_) => FailureCode::Unknown,
        InstanceError::Lifecycle(error) => lifecycle_failure_code(error),
        InstanceError::Ledger(_) => FailureCode::Ledger,
    }
}

/// Returns the wire value of a failure code.
#[must_use]
pub const fn failure_code(code: FailureCode) -> u16 {
    code as u16
}

pub(super) fn array<const N: usize>(slice: &[u8]) -> [u8; N] {
    let mut out = [0; N];
    out.copy_from_slice(slice);
    out
}

#[cfg(test)]
mod tests;
