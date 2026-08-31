//! Why a lifecycle operation was refused.

use std::fmt;

use crate::{ClaimError, InstanceId, LedgerError, LifecycleError, OperationId, TransferFailure};

/// The typed refusal of one Instance lifecycle operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstanceError {
    /// The claim behind the Launch was refused; no Instance was created.
    Claim(ClaimError),
    /// Authority could not be transferred; the worker was destroyed.
    Transfer(TransferFailure),
    /// The Instance identity is already live under a different operation.
    ///
    /// One Instance has exactly one owner, so a second operation may not adopt it; the
    /// presenting client either replays its own operation or chooses a fresh Instance.
    Occupied {
        /// The Instance.
        instance: InstanceId,
        /// The operation that holds it.
        holder: OperationId,
        /// The operation that presented it.
        presented: OperationId,
    },
    /// The operation's worker is terminal, so its Launch did not succeed and the Instance
    /// cannot be revived under the same identity.
    Terminated(InstanceId),
    /// No live Instance and no durable record carries this identity.
    Unknown(InstanceId),
    /// The pool refused the terminal transition.
    Lifecycle(LifecycleError),
    /// The durable record could not be read, so no terminal claim may be made.
    Ledger(LedgerError),
}

impl fmt::Display for InstanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Claim(error) => write!(formatter, "claim refused: {error}"),
            Self::Transfer(failure) => write!(formatter, "transfer refused: {failure}"),
            Self::Occupied {
                instance,
                holder,
                presented,
            } => write!(
                formatter,
                "{instance:?} is held by {holder:?} and was presented by {presented:?}"
            ),
            Self::Terminated(instance) => write!(formatter, "{instance:?} is terminal"),
            Self::Unknown(instance) => write!(formatter, "{instance:?} is unknown"),
            Self::Lifecycle(error) => write!(formatter, "lifecycle refused: {error}"),
            Self::Ledger(error) => write!(formatter, "ledger: {error}"),
        }
    }
}

impl std::error::Error for InstanceError {}
