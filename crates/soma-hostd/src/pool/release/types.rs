//! Destroy reasons, release evidence, and lifecycle refusals.

use std::fmt;

use crate::{
    DestroyOutcome, LeaseGeneration, LedgerError, Phase, ResourceRelease, StartFault, StateRace,
    WorkerId,
};

/// Why a worker is destroyed; recorded in the ledger.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DestroyReason {
    /// A transfer step faulted.
    TransferFault = 1,
    /// The claim deadline passed before the transfer completed.
    ClaimDeadline = 2,
    /// The Instance was released.
    Released = 3,
    /// A sterile worker was evicted.
    Evicted = 4,
    /// The Instance refused to start.
    StartFault = 5,
    /// Reconciliation after a restart.
    Reconcile = 6,
    /// The ledger refused a record.
    Ledger = 7,
    /// The claim grant was dropped without a transfer.
    Dropped = 8,
    /// The transfer intent did not match the claim.
    IntentMismatch = 9,
}

/// What releasing one worker did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReleaseEvidence {
    /// The worker.
    pub worker: WorkerId,
    /// The lease generation.
    pub lease_generation: LeaseGeneration,
    /// The reason.
    pub reason: DestroyReason,
    /// The process teardown.
    pub destroyed: DestroyOutcome,
    /// The resource release.
    pub released: ResourceRelease,
    /// Whether every ledger record was written.
    pub ledger: bool,
}

/// Why a lifecycle call was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleError {
    /// No owned worker has this identity.
    Unknown(WorkerId),
    /// The worker is in the wrong phase.
    Phase {
        /// The worker.
        worker: WorkerId,
        /// Its phase.
        phase: Phase,
    },
    /// The Instance refused to start; the worker was destroyed.
    Start(StartFault),
    /// The ledger refused a record.
    Ledger(LedgerError),
    /// The worker's state moved under the call.
    State(StateRace),
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(worker) => write!(formatter, "{worker:?} is not owned"),
            Self::Phase { worker, phase } => write!(formatter, "{worker:?} is {phase:?}"),
            Self::Start(fault) => write!(formatter, "start refused: {fault:?}"),
            Self::Ledger(error) => write!(formatter, "ledger: {error}"),
            Self::State(race) => write!(formatter, "state race: {race:?}"),
        }
    }
}

impl std::error::Error for LifecycleError {}
