//! The transfer sequence over a claimed worker.

mod frames;

use std::{fmt, time::Instant};

use frames::{frames, fresh_entropy};

use crate::{
    AssignedResources, AssignmentIntent, Claimed, Claiming, DestroyReason, Pool, Record,
    RecordKind, Reservation, ResourceBroker, ResourceRefs, StepAck, TransferEvidence,
    TransferFault, TransferFrame, TransferStep, Worker, WorkerHandle, WorkerId, WorkerIdentity,
    WorkerLauncher,
    pool::{
        Owned, OwnedWorker, Prepared,
        release::{Holdings, Resources},
        transfer::Disposition,
    },
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

/// The live state of one transfer.
struct Attempt<H> {
    worker: Worker<Claiming>,
    handle: H,
    identity: WorkerIdentity,
    refs: ResourceRefs,
    reservation: Option<Reservation>,
    won_at: Instant,
}

impl<L: WorkerLauncher, R: ResourceBroker> Pool<L, R> {
    /// Transfers fresh authority for `intent` into the claimed worker exactly once.
    ///
    /// # Errors
    ///
    /// Returns the typed failure after the worker was destroyed.
    pub fn transfer(
        &self,
        claimed: Claimed<'_, L, R>,
        intent: &AssignmentIntent,
    ) -> Result<TransferEvidence, TransferFailure> {
        let (mut attempt, resources) = self.admit(claimed, intent)?;
        let id = attempt.worker.id();
        let generation = attempt.worker.generation();
        let won_at = attempt.won_at;
        let launch = resources.network.launch;
        let seed = match fresh_entropy() {
            Ok(seed) => seed,
            Err(fault) => return Err(self.abort(attempt, None, fault)),
        };
        let mut steps = 0;
        for frame in frames(id, generation, intent, seed, resources) {
            let step = frame.step();
            if let Err(fault) = self.deliver(&mut attempt, intent, frame) {
                return Err(self.abort(attempt, Some(step), fault));
            }
            steps += 1;
        }
        self.commit(attempt, intent)?;
        Ok(TransferEvidence {
            worker: id,
            lease_generation: generation,
            instance: intent.instance,
            operation: intent.operation,
            launch,
            steps,
            elapsed: won_at.elapsed(),
        })
    }

    /// Checks the claim, assigns resources, and returns the live attempt.
    fn admit(
        &self,
        mut claimed: Claimed<'_, L, R>,
        intent: &AssignmentIntent,
    ) -> Result<(Attempt<L::Handle>, AssignedResources), TransferFailure> {
        let (Some(worker), Some(prepared)) = (claimed.worker.take(), claimed.prepared.take())
        else {
            unreachable!("a live grant always holds its worker and payload");
        };
        let won_at = claimed.won_at;
        let fingerprint = claimed.fingerprint;
        let reservation = claimed.reservation.take();
        drop(claimed);
        let Prepared {
            handle,
            sterile,
            identity,
            ..
        } = prepared;
        let id = worker.id();
        let refusal = if intent.fingerprint() != fingerprint {
            Some((DestroyReason::IntentMismatch, TransferFault::Rejected))
        } else if won_at.elapsed() > self.limits().claim_deadline {
            Some((DestroyReason::ClaimDeadline, TransferFault::ClaimDeadline))
        } else {
            None
        };
        if let Some((reason, fault)) = refusal {
            let held = Holdings {
                handle: Some(handle),
                identity,
                resources: Resources::Sterile(sterile),
                reservation,
            };
            let disposition = self.destroy_claiming(worker, held, reason);
            return Err(failure(id, None, fault, disposition));
        }
        match self.broker().assign(sterile, intent) {
            Ok(resources) => Ok((
                Attempt {
                    worker,
                    handle,
                    identity,
                    refs: resources.refs(),
                    reservation,
                    won_at,
                },
                resources,
            )),
            Err(fault) => {
                let held = Holdings {
                    handle: Some(handle),
                    identity,
                    resources: Resources::None,
                    reservation,
                };
                let disposition = self.destroy_claiming(worker, held, DestroyReason::TransferFault);
                Err(failure(
                    id,
                    None,
                    TransferFault::Resource(fault),
                    disposition,
                ))
            }
        }
    }

    fn deliver(
        &self,
        attempt: &mut Attempt<L::Handle>,
        intent: &AssignmentIntent,
        frame: TransferFrame,
    ) -> Result<(), TransferFault> {
        let step = frame.step();
        let id = attempt.worker.id();
        let generation = attempt.worker.generation();
        let key = self.digest();
        let result = if attempt.won_at.elapsed() > self.limits().claim_deadline {
            Err(TransferFault::ClaimDeadline)
        } else {
            attempt.handle.deliver(frame).and_then(|StepAck::Accepted| {
                let record = Record::new(RecordKind::TransferStep, id, generation, key)
                    .detail(step.code())
                    .operation(intent.operation)
                    .instance(intent.instance)
                    .identity(attempt.identity)
                    .resources(attempt.refs);
                self.record(&record)
                    .map(|_| ())
                    .map_err(TransferFault::Ledger)
            })
        };
        if result.is_err() {
            let _ = self.record(
                &Record::new(RecordKind::TransferFault, id, generation, key)
                    .detail(step.code())
                    .operation(intent.operation)
                    .instance(intent.instance),
            );
        }
        result
    }

    fn abort(
        &self,
        mut attempt: Attempt<L::Handle>,
        step: Option<TransferStep>,
        fault: TransferFault,
    ) -> TransferFailure {
        let id = attempt.worker.id();
        let reason = if fault == TransferFault::ClaimDeadline {
            DestroyReason::ClaimDeadline
        } else {
            DestroyReason::TransferFault
        };
        let held = Holdings {
            handle: Some(attempt.handle),
            identity: attempt.identity,
            resources: Resources::Assigned(attempt.refs),
            reservation: attempt.reservation.take(),
        };
        let disposition = self.destroy_claiming(attempt.worker, held, reason);
        failure(id, step, fault, disposition)
    }

    fn commit(
        &self,
        mut attempt: Attempt<L::Handle>,
        intent: &AssignmentIntent,
    ) -> Result<(), TransferFailure> {
        let id = attempt.worker.id();
        let generation = attempt.worker.generation();
        let mut reservation = attempt.reservation.take();
        let assigned = match attempt.worker.assign() {
            Ok(assigned) => assigned,
            Err(race) => {
                let disposition = Disposition {
                    destroyed: attempt.handle.destroy(),
                    released: self.broker().release(&attempt.refs),
                };
                self.release_capacity(reservation);
                let fault = TransferFault::State(race);
                return Err(failure(id, Some(TransferStep::Commit), fault, disposition));
            }
        };
        self.capacity_launched(&mut reservation);
        let owned = Owned {
            worker: OwnedWorker::Assigned(assigned),
            handle: Some(attempt.handle),
            identity: attempt.identity,
            refs: attempt.refs,
            instance: intent.instance,
            operation: intent.operation,
            reservation,
        };
        let record = Record::new(RecordKind::Assigned, id, generation, self.digest())
            .operation(intent.operation)
            .instance(intent.instance)
            .identity(attempt.identity)
            .resources(attempt.refs);
        if let Err(error) = self.record(&record) {
            let evidence = self.destroy_owned(owned, DestroyReason::Ledger);
            let disposition = Disposition {
                destroyed: evidence.destroyed,
                released: evidence.released,
            };
            let fault = TransferFault::Ledger(error);
            return Err(failure(id, Some(TransferStep::Commit), fault, disposition));
        }
        self.owned
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, owned);
        Ok(())
    }
}

const fn failure(
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
