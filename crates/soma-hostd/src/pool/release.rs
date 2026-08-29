//! Teardown of workers in every phase: the single-use worker is destroyed, its resources
//! released by reference, and the ledger closed; nothing ever returns to `Sterile`.

mod types;

pub use types::{DestroyReason, LifecycleError, ReleaseEvidence};

use crate::{
    Claiming, DestroyOutcome, Destroying, Phase, Pool, Record, RecordKind, Removal, Reservation,
    ResourceBroker, ResourceRefs, ResourceRelease, Worker, WorkerHandle, WorkerId, WorkerIdentity,
    WorkerLauncher,
    pool::{Owned, OwnedWorker, Prepared, transfer::Disposition},
};

/// Which broker resources a worker still holds when it is destroyed.
pub(crate) enum Resources<S> {
    Sterile(S),
    Assigned(ResourceRefs),
    None,
}

/// Everything one destroyed worker still holds: its process, its broker resources, and the
/// capacity reservation its Instance was admitted under.
pub(crate) struct Holdings<H, S> {
    pub(crate) handle: Option<H>,
    pub(crate) identity: WorkerIdentity,
    pub(crate) resources: Resources<S>,
    pub(crate) reservation: Option<Reservation>,
}

impl<L: WorkerLauncher, R: ResourceBroker> Pool<L, R> {
    /// Starts an assigned worker's Instance.
    ///
    /// # Errors
    ///
    /// Returns the typed refusal; a start fault destroys the worker.
    pub fn start(&self, worker: WorkerId) -> Result<(), LifecycleError> {
        let mut owned = self
            .owned
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entry) = owned.get_mut(&worker) else {
            return Err(LifecycleError::Unknown(worker));
        };
        let OwnedWorker::Assigned(_) = &entry.worker else {
            return Err(LifecycleError::Phase {
                worker,
                phase: Phase::Running,
            });
        };
        let started = entry.handle.as_mut().map_or(Ok(()), WorkerHandle::start);
        let Some(mut taken) = owned.remove(&worker) else {
            return Err(LifecycleError::Unknown(worker));
        };
        drop(owned);
        if let Err(fault) = started {
            let _ = self.destroy_owned(taken, DestroyReason::StartFault);
            return Err(LifecycleError::Start(fault));
        }
        let OwnedWorker::Assigned(assigned) = taken.worker else {
            return Err(LifecycleError::Unknown(worker));
        };
        let running = assigned.run().map_err(LifecycleError::State)?;
        let record = Record::new(
            RecordKind::Running,
            worker,
            running.generation(),
            self.digest(),
        )
        .operation(taken.operation)
        .instance(taken.instance)
        .resources(taken.refs)
        .identity(taken.identity);
        taken.worker = OwnedWorker::Running(running);
        let recorded = self.record(&record);
        self.owned
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(worker, taken);
        recorded.map(|_| ()).map_err(LifecycleError::Ledger)
    }

    /// Releases an assigned or running worker; it is destroyed, never reused.
    ///
    /// # Errors
    ///
    /// Returns [`LifecycleError::Unknown`] when the pool owns no such worker.
    pub fn release(&self, worker: WorkerId) -> Result<ReleaseEvidence, LifecycleError> {
        let owned = self
            .owned
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&worker)
            .ok_or(LifecycleError::Unknown(worker))?;
        Ok(self.destroy_owned(owned, DestroyReason::Released))
    }

    /// Evicts every sterile worker, for example when the Generation is retired.
    pub fn evict_sterile(&self) -> Vec<ReleaseEvidence> {
        let mut evidence = Vec::new();
        for slot in self.slots() {
            let Some(destroying) = slot.try_evict() else {
                continue;
            };
            let prepared = self
                .prepared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&slot.id());
            let (handle, identity, resources) = match prepared {
                Some(Prepared {
                    handle,
                    sterile,
                    identity,
                    ..
                }) => (Some(handle), identity, Resources::Sterile(sterile)),
                None => (None, absent(), Resources::None),
            };
            evidence.push(self.teardown(
                destroying,
                handle,
                identity,
                resources,
                DestroyReason::Evicted,
            ));
        }
        evidence
    }

    pub(crate) fn destroy_claiming(
        &self,
        worker: Worker<Claiming>,
        held: Holdings<L::Handle, R::Sterile>,
        reason: DestroyReason,
    ) -> Disposition {
        let Holdings {
            handle,
            identity,
            resources,
            reservation,
        } = held;
        let disposition = match worker.destroy() {
            Ok(destroying) => {
                let evidence = self.teardown(destroying, handle, identity, resources, reason);
                Disposition {
                    destroyed: evidence.destroyed,
                    released: evidence.released,
                }
            }
            Err(_) => self.orphan(handle, resources),
        };
        self.release_capacity(reservation);
        disposition
    }

    pub(crate) fn destroy_owned(
        &self,
        mut owned: Owned<L::Handle>,
        reason: DestroyReason,
    ) -> ReleaseEvidence {
        self.release_capacity(owned.reservation.take());
        let (id, generation, destroying) = match owned.worker {
            OwnedWorker::Assigned(worker) => (worker.id(), worker.generation(), worker.destroy()),
            OwnedWorker::Running(worker) => (worker.id(), worker.generation(), worker.destroy()),
        };
        let Ok(destroying) = destroying else {
            let disposition =
                self.orphan(owned.handle, Resources::<R::Sterile>::Assigned(owned.refs));
            return ReleaseEvidence {
                worker: id,
                lease_generation: generation,
                reason,
                destroyed: disposition.destroyed,
                released: disposition.released,
                ledger: false,
            };
        };
        self.teardown(
            destroying,
            owned.handle,
            owned.identity,
            Resources::Assigned(owned.refs),
            reason,
        )
    }

    fn teardown(
        &self,
        worker: Worker<Destroying>,
        handle: Option<L::Handle>,
        identity: WorkerIdentity,
        resources: Resources<R::Sterile>,
        reason: DestroyReason,
    ) -> ReleaseEvidence {
        let id = worker.id();
        let generation = worker.generation();
        let mut ledger = self
            .record(
                &Record::new(RecordKind::Destroying, id, generation, self.digest())
                    .detail(reason as u8)
                    .identity(identity),
            )
            .is_ok();
        let destroyed = match handle {
            Some(handle) => handle.destroy(),
            None if identity.process != 0 => self.launcher().terminate(identity),
            None => no_process(),
        };
        let released = self.release_resources(resources);
        ledger &= self
            .record(
                &Record::new(RecordKind::Dead, id, generation, self.digest()).detail(reason as u8),
            )
            .is_ok();
        let _ = worker.finish();
        ReleaseEvidence {
            worker: id,
            lease_generation: generation,
            reason,
            destroyed,
            released,
            ledger,
        }
    }

    fn release_resources(&self, resources: Resources<R::Sterile>) -> ResourceRelease {
        match resources {
            Resources::Sterile(sterile) => self.broker().release_sterile(sterile),
            Resources::Assigned(refs) => self.broker().release(&refs),
            Resources::None => ResourceRelease {
                disk: Removal::AlreadyAbsent,
                network: Removal::AlreadyAbsent,
                complete: true,
            },
        }
    }

    fn orphan(&self, handle: Option<L::Handle>, resources: Resources<R::Sterile>) -> Disposition {
        Disposition {
            destroyed: Self::destroy_handle(handle).unwrap_or_else(no_process),
            released: self.release_resources(resources),
        }
    }
}

const fn no_process() -> DestroyOutcome {
    DestroyOutcome {
        process: Removal::AlreadyAbsent,
        cgroup: Removal::AlreadyAbsent,
        complete: true,
    }
}

pub(crate) const fn absent() -> WorkerIdentity {
    WorkerIdentity {
        process: 0,
        token: [0; 16],
    }
}
