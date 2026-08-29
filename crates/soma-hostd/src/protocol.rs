//! The bounded typed daemon protocol carried in `SOCK_SEQPACKET` frames.
//!
//! One request frame produces one reply frame; no frame carries a path, a descriptor number,
//! or free text, and every failure is a stable code.

mod reply;

use std::fmt;

pub use reply::Reply;

use soma_netd::{MAX_ENCODED_INTENT, NetworkIntent};

use crate::{
    ClaimError, InstanceId, LaunchMaterialHandle, LifecycleError, OperationId, TransferFailure,
    WorkerId,
};

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

/// One client request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Request {
    /// Claim one sterile worker and transfer fresh authority for the Instance.
    Claim {
        /// The operation.
        operation: OperationId,
        /// The Instance.
        instance: InstanceId,
        /// The vsock CID.
        vsock_cid: u32,
        /// Nanoseconds the Instance may live.
        deadline_nanos: u64,
        /// The sealed launch material.
        launch_material: LaunchMaterialHandle,
        /// The admitted network intent.
        intent: NetworkIntent,
    },
    /// Release one assigned or running worker.
    Release {
        /// The worker.
        worker: WorkerId,
    },
    /// Inspect one worker.
    Inspect {
        /// The worker.
        worker: WorkerId,
    },
    /// Reconcile the ledger.
    Reconcile,
}

impl Request {
    /// Encodes the request.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MAX_FRAME);
        match self {
            Self::Claim {
                operation,
                instance,
                vsock_cid,
                deadline_nanos,
                launch_material,
                intent,
            } => {
                out.push(1);
                out.extend_from_slice(operation.as_bytes());
                out.extend_from_slice(instance.as_bytes());
                out.extend_from_slice(&vsock_cid.to_be_bytes());
                out.extend_from_slice(&deadline_nanos.to_be_bytes());
                out.extend_from_slice(launch_material.as_bytes());
                out.extend_from_slice(&intent.encode());
            }
            Self::Release { worker } => {
                out.push(2);
                out.extend_from_slice(worker.as_bytes());
            }
            Self::Inspect { worker } => {
                out.push(3);
                out.extend_from_slice(worker.as_bytes());
            }
            Self::Reconcile => out.push(4),
        }
        out
    }

    /// Decodes one exact request frame.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] for any malformed frame.
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.is_empty() || bytes.len() > MAX_FRAME {
            return Err(ProtocolError("frame length"));
        }
        match bytes[0] {
            1 if bytes.len() > CLAIM_HEADER => Ok(Self::Claim {
                operation: OperationId::new(array(&bytes[1..17]))
                    .map_err(|_| ProtocolError("operation"))?,
                instance: InstanceId::new(array(&bytes[17..33]))
                    .map_err(|_| ProtocolError("instance"))?,
                vsock_cid: u32::from_be_bytes(array(&bytes[33..37])),
                deadline_nanos: u64::from_be_bytes(array(&bytes[37..45])),
                launch_material: LaunchMaterialHandle::new(array(&bytes[45..77]))
                    .map_err(|_| ProtocolError("launch material"))?,
                intent: NetworkIntent::decode(&bytes[CLAIM_HEADER..])
                    .map_err(|_| ProtocolError("intent"))?,
            }),
            2 | 3 if bytes.len() == 17 => {
                let worker =
                    WorkerId::new(array(&bytes[1..17])).map_err(|_| ProtocolError("worker"))?;
                Ok(if bytes[0] == 2 {
                    Self::Release { worker }
                } else {
                    Self::Inspect { worker }
                })
            }
            4 if bytes.len() == 1 => Ok(Self::Reconcile),
            _ => Err(ProtocolError("request")),
        }
    }
}

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
    /// Changed intent under a replayed operation.
    Conflict = 4,
    /// Claim deadline.
    Deadline = 5,
    /// Ledger failure.
    Ledger = 6,
    /// Construction failure.
    Construction = 7,
    /// Transfer failure.
    Transfer = 8,
    /// Unknown worker.
    Unknown = 9,
    /// Wrong phase.
    Phase = 10,
    /// Internal invariant.
    Invariant = 11,
}

/// Maps a claim failure onto its code.
#[must_use]
pub const fn claim_failure_code(error: &ClaimError) -> FailureCode {
    match error {
        ClaimError::Exhausted(_) => FailureCode::Exhausted,
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
