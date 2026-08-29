//! The single-winner, idempotent claim.
//!
//! The registry serializes bookkeeping per `OperationId`: a replay with the same fingerprint
//! returns the identical outcome, a changed fingerprint conflicts, and a concurrent replay
//! waits at most the claim deadline for the in-flight attempt.
//! Ownership itself is decided by the one compare-and-swap in [`Slot::try_claim`], which the
//! replenisher, the reconciler, and every other claimer contend on without the registry lock.

mod error;
mod registry;

use std::time::Instant;

pub use error::ClaimError;

pub(crate) use registry::Registry;

use crate::{
    Claiming, DestroyReason, Exhausted, ExhaustedBehavior, LeaseGeneration, OperationId, Pool,
    Record, RecordKind, RequestFingerprint, Reservation, ResourceBroker, Slot, Worker, WorkerId,
    WorkerLauncher,
    pool::{
        Prepared,
        release::{Holdings, Resources},
    },
};

/// Whether the claim took a prepared worker or built one inline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ClaimClass {
    /// A sterile worker prepared before the request.
    Prepared = 1,
    /// A worker constructed inline because the pool was empty; a separate measurement class.
    OnDemand = 2,
}

impl ClaimClass {
    /// Decodes one class from its ledger detail byte.
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Prepared),
            2 => Some(Self::OnDemand),
            _ => None,
        }
    }
}

/// The identical result every replay of one operation receives.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClaimOutcome {
    /// The worker.
    pub worker: WorkerId,
    /// The lease generation the claim won.
    pub lease_generation: LeaseGeneration,
    /// The operation.
    pub operation: OperationId,
    /// The class.
    pub class: ClaimClass,
}

/// The fresh winner's grant: the claimed worker and its sterile payload, which the caller
/// must transfer before the claim deadline; dropping it destroys the worker.
pub struct Claimed<'p, L: WorkerLauncher, R: ResourceBroker> {
    pub(crate) pool: &'p Pool<L, R>,
    pub(crate) worker: Option<Worker<Claiming>>,
    pub(crate) prepared: Option<Prepared<L::Handle, R::Sterile>>,
    pub(crate) reservation: Option<Reservation>,
    pub(crate) won_at: Instant,
    pub(crate) fingerprint: RequestFingerprint,
    outcome: ClaimOutcome,
}

impl<L: WorkerLauncher, R: ResourceBroker> Claimed<'_, L, R> {
    /// Returns the outcome.
    #[must_use]
    pub const fn outcome(&self) -> ClaimOutcome {
        self.outcome
    }

    /// Returns when the claim won.
    #[must_use]
    pub const fn won_at(&self) -> Instant {
        self.won_at
    }
}

impl<L: WorkerLauncher, R: ResourceBroker> Drop for Claimed<'_, L, R> {
    fn drop(&mut self) {
        if let (Some(worker), Some(prepared)) = (self.worker.take(), self.prepared.take()) {
            let _ = self.pool.destroy_claiming(
                worker,
                Holdings {
                    handle: Some(prepared.handle),
                    identity: prepared.identity,
                    resources: Resources::Sterile(prepared.sterile),
                    reservation: self.reservation.take(),
                },
                DestroyReason::Dropped,
            );
        }
    }
}

/// One successful claim: the identical outcome plus the grant for the fresh winner only.
pub struct Claim<'p, L: WorkerLauncher, R: ResourceBroker> {
    /// The outcome every replay receives.
    pub outcome: ClaimOutcome,
    /// The transfer grant; `None` on replay.
    pub grant: Option<Claimed<'p, L, R>>,
}

impl<L: WorkerLauncher, R: ResourceBroker> Pool<L, R> {
    /// Claims exactly one sterile worker for `operation`.
    ///
    /// The claim reserves every capacity dimension of the pool's Machine shape atomically
    /// before it wins a slot, so a worker is granted only for an admitted Instance.
    ///
    /// # Errors
    ///
    /// Returns the typed rejection; an exhausted pool never queues.
    pub fn claim(
        &self,
        operation: OperationId,
        fingerprint: RequestFingerprint,
    ) -> Result<Claim<'_, L, R>, ClaimError> {
        let started = Instant::now();
        if let Some(outcome) = self.reserve_binding(operation, fingerprint, started)? {
            return Ok(Claim {
                outcome,
                grant: None,
            });
        }
        let reservation = match self.reserve_capacity() {
            Ok(reservation) => Some(reservation),
            Err(rejection) => {
                self.clear_binding(operation);
                return Err(ClaimError::Capacity(rejection));
            }
        };
        let (worker, class) = match self.acquire() {
            Ok(won) => won,
            Err(error) => {
                self.release_capacity(reservation);
                self.clear_binding(operation);
                return Err(error);
            }
        };
        let Some(prepared) = self
            .prepared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&worker.id())
        else {
            let id = worker.id();
            let _ = self.destroy_claiming(
                worker,
                Holdings {
                    handle: None,
                    identity: no_identity(),
                    resources: Resources::None,
                    reservation,
                },
                DestroyReason::Ledger,
            );
            self.clear_binding(operation);
            return Err(ClaimError::MissingPayload(id));
        };
        let outcome = ClaimOutcome {
            worker: worker.id(),
            lease_generation: worker.generation(),
            operation,
            class,
        };
        let record = Record::new(
            RecordKind::Claiming,
            outcome.worker,
            outcome.lease_generation,
            self.digest(),
        )
        .detail(class as u8)
        .operation(operation)
        .fingerprint(fingerprint)
        .identity(prepared.identity)
        .resources(prepared.refs);
        if let Err(error) = self.record(&record) {
            let _ = self.destroy_claiming(
                worker,
                Holdings {
                    handle: Some(prepared.handle),
                    identity: prepared.identity,
                    resources: Resources::Sterile(prepared.sterile),
                    reservation,
                },
                DestroyReason::Ledger,
            );
            self.clear_binding(operation);
            return Err(ClaimError::Ledger(error));
        }
        self.finish_binding(operation, fingerprint, outcome);
        Ok(Claim {
            outcome,
            grant: Some(Claimed {
                pool: self,
                worker: Some(worker),
                prepared: Some(prepared),
                reservation,
                won_at: started,
                fingerprint,
                outcome,
            }),
        })
    }

    /// Wins one sterile slot, constructing a worker inline when the pool allows it.
    fn acquire(&self) -> Result<(Worker<Claiming>, ClaimClass), ClaimError> {
        if let Some(worker) = self.slots().iter().find_map(Slot::try_claim) {
            return Ok((worker, ClaimClass::Prepared));
        }
        match self.limits().exhausted {
            ExhaustedBehavior::Reject => Err(ClaimError::Exhausted(self.exhausted())),
            ExhaustedBehavior::ConstructInline => self
                .construct_one()
                .map_err(ClaimError::Construction)
                .and_then(|id| {
                    self.find_slot(id)
                        .and_then(|slot| slot.try_claim())
                        .ok_or_else(|| ClaimError::Exhausted(self.exhausted()))
                })
                .map(|worker| (worker, ClaimClass::OnDemand)),
        }
    }

    pub(crate) fn exhausted(&self) -> Exhausted {
        Exhausted {
            key: self.digest(),
            occupancy: self.occupancy(),
            max: self.limits().max,
            behavior: self.limits().exhausted,
        }
    }
}

const fn no_identity() -> crate::WorkerIdentity {
    crate::WorkerIdentity {
        process: 0,
        token: [0; 16],
    }
}
