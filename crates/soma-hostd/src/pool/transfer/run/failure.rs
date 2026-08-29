//! The typed outcome of a failed transfer.

use std::fmt;

use crate::{
    Claimed, DestroyReason, Disposition, ResourceBroker, TransferFault, TransferStep, WorkerId,
    WorkerLauncher,
    pool::release::{Holdings, Resources},
};

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

/// Destroys a grant this pool did not issue through the pool that did.
///
/// Assigning one pool's broker resources to another pool's slot would write records for a
/// worker the receiving ledger never constructed, which no later fold could project.
pub(super) fn refuse_foreign<L: WorkerLauncher, R: ResourceBroker>(
    mut claimed: Claimed<'_, L, R>,
) -> TransferFailure {
    let issuer = claimed.pool;
    let (Some(worker), Some(prepared)) = (claimed.worker.take(), claimed.prepared.take()) else {
        unreachable!("a live grant always holds its worker and payload");
    };
    let id = worker.id();
    let held = Holdings {
        handle: Some(prepared.handle),
        identity: prepared.identity,
        resources: Resources::Sterile(prepared.sterile),
        reservation: claimed.reservation.take(),
    };
    let disposition = issuer.destroy_claiming(worker, held, DestroyReason::ForeignPool);
    failure(id, None, TransferFault::ForeignPool, disposition)
}
