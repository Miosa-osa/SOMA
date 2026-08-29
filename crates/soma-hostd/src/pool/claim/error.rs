//! Why a claim was refused; nothing was ever queued behind one.

use std::{fmt, time::Duration};

use crate::{
    CapacityRejection, ConstructionFailure, Exhausted, LedgerError, OperationId, Overloaded,
    RequestFingerprint, WorkerId,
};

/// Why a claim was refused; nothing was queued.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaimError {
    /// No sterile worker.
    Exhausted(Exhausted),
    /// A capacity gate refused the Instance this claim would admit.
    Capacity(CapacityRejection),
    /// A bounded structure is full.
    Overloaded(Overloaded),
    /// The operation already claimed with a different intent.
    OperationConflict {
        /// The operation.
        operation: OperationId,
        /// The fingerprint the ledger holds.
        recorded: RequestFingerprint,
        /// The fingerprint presented now.
        presented: RequestFingerprint,
    },
    /// An in-flight claim for the same operation did not finish within the deadline.
    Deadline {
        /// The operation.
        operation: OperationId,
        /// How long the caller waited.
        waited: Duration,
    },
    /// The claim could not be recorded; the worker was destroyed.
    Ledger(LedgerError),
    /// Inline construction failed.
    Construction(ConstructionFailure),
    /// A claimed slot had no prepared payload; the worker was destroyed.
    MissingPayload(WorkerId),
}

impl fmt::Display for ClaimError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exhausted(exhausted) => write!(formatter, "{exhausted}"),
            Self::Capacity(rejection) => write!(formatter, "capacity refused: {rejection}"),
            Self::Overloaded(overloaded) => write!(formatter, "{overloaded}"),
            Self::OperationConflict { operation, .. } => {
                write!(formatter, "{operation:?} replayed with a different intent")
            }
            Self::Deadline { operation, waited } => {
                write!(
                    formatter,
                    "{operation:?} waited {waited:?} for an in-flight claim"
                )
            }
            Self::Ledger(error) => write!(formatter, "claim not recorded: {error}"),
            Self::Construction(failure) => write!(formatter, "inline construction: {failure}"),
            Self::MissingPayload(worker) => write!(formatter, "{worker:?} had no payload"),
        }
    }
}

impl std::error::Error for ClaimError {}
