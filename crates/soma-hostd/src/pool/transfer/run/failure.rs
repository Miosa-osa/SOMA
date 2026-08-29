//! The typed outcome of a failed transfer.

use std::fmt;

use crate::{Disposition, TransferFault, TransferStep, WorkerId};

/// A failed transfer: the worker was destroyed and never returned to the pool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransferFailure {
    /// The worker.
    pub worker: WorkerId,
    /// The step that failed, when a step was reached.
    pub step: Option<TransferStep>,
    /// The fault.
    pub fault: TransferFault,
    /// What teardown did.
    pub disposition: Disposition,
}

impl fmt::Display for TransferFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:?} transfer failed at {:?}: {}",
            self.worker, self.step, self.fault
        )
    }
}

impl std::error::Error for TransferFailure {}

pub(super) const fn failure(
    worker: WorkerId,
    step: Option<TransferStep>,
    fault: TransferFault,
    disposition: Disposition,
) -> TransferFailure {
    TransferFailure {
        worker,
        step,
        fault,
        disposition,
    }
}
