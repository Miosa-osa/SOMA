//! Bounded background construction toward the target.
//!
//! Every construction reserves one of `replenish_concurrency` slots before it starts, runs
//! inside the construction deadline, and either publishes a sterile worker with its ledger
//! records or closes the worker as dead; nothing waits in a queue.

mod background;

use std::{
    fmt,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use crate::{
    ConstructFault, Constructing, LedgerError, OverloadGate, Overloaded, Pool, Record, RecordKind,
    ResourceBroker, ResourceFault, Slot, StateRace, Worker, WorkerHandle, WorkerId, WorkerLauncher,
    pool::Prepared,
};

/// What bounded one replenishment pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplenishLimit {
    /// Every construction slot was busy.
    Concurrency,
    /// The pool holds `max` live workers.
    PoolMaximum,
    /// The ledger has unreconciled suspects.
    Unreconciled,
}

/// The result of one replenishment pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplenishReport {
    /// Workers the target still lacked.
    pub deficit: usize,
    /// Constructions started by this pass.
    pub spawned: usize,
    /// Constructions running after this pass.
    pub in_flight: usize,
    /// What stopped the pass from covering the deficit.
    pub limited_by: Option<ReplenishLimit>,
    /// Whether the pool ended the pass with fewer sterile workers than its minimum.
    pub urgent: bool,
}

/// Why one construction failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConstructionFault {
    /// The pool or its concurrency is full.
    Overloaded(Overloaded),
    /// The ledger has unreconciled suspects.
    Unreconciled,
    /// The launcher failed.
    Launcher(ConstructFault),
    /// The resource broker failed.
    Resources(ResourceFault),
    /// The ledger refused a record.
    Ledger(LedgerError),
    /// The slot moved under construction.
    State(StateRace),
    /// The construction deadline passed after the parts existed.
    Deadline {
        /// The budget.
        budget: Duration,
        /// The observed cost.
        elapsed: Duration,
    },
}

/// One failed construction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConstructionFailure {
    /// The worker, once a slot existed.
    pub worker: Option<WorkerId>,
    /// The fault.
    pub fault: ConstructionFault,
}

impl fmt::Display for ConstructionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {:?}", self.worker, self.fault)
    }
}

impl std::error::Error for ConstructionFailure {}

impl<L: WorkerLauncher, R: ResourceBroker> Pool<L, R> {
    /// Constructs sequentially in the calling thread until the target is met.
    ///
    /// # Errors
    ///
    /// Returns the first failure with the number of workers constructed before it.
    pub fn replenish_blocking(&self) -> Result<usize, ConstructionFailure> {
        let mut built = 0;
        while self.occupancy().sterile + self.in_flight.load(Ordering::Acquire)
            < self.limits().target
        {
            self.construct_one()?;
            built += 1;
        }
        Ok(built)
    }

    /// Constructs exactly one sterile worker inside one concurrency slot.
    ///
    /// # Errors
    ///
    /// Returns the typed failure; nothing is left behind.
    pub fn construct_one(&self) -> Result<WorkerId, ConstructionFailure> {
        let limit = self.limits().replenish_concurrency;
        let reserved =
            self.in_flight
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    (current < limit).then_some(current + 1)
                });
        if let Err(current) = reserved {
            return Err(ConstructionFailure {
                worker: None,
                fault: ConstructionFault::Overloaded(Overloaded {
                    gate: OverloadGate::ReplenishConcurrency,
                    current,
                    limit,
                }),
            });
        }
        let result = self.construct_reserved();
        self.in_flight.fetch_sub(1, Ordering::AcqRel);
        result
    }

    fn construct_reserved(&self) -> Result<WorkerId, ConstructionFailure> {
        if self.needs_reconcile() {
            return Err(ConstructionFailure {
                worker: None,
                fault: ConstructionFault::Unreconciled,
            });
        }
        let id = self.fresh_worker_id();
        let slot = self
            .add_slot(Slot::new(id, self.digest()))
            .map_err(|overloaded| ConstructionFailure {
                worker: None,
                fault: ConstructionFault::Overloaded(overloaded),
            })?;
        let worker = Worker::<Constructing>::open(slot);
        let failure = |fault| ConstructionFailure {
            worker: Some(id),
            fault,
        };
        let key = self.digest();
        if let Err(error) = self.record(&Record::new(
            RecordKind::Constructing,
            id,
            worker.generation(),
            key,
        )) {
            let _ = worker.abort();
            return Err(failure(ConstructionFault::Ledger(error)));
        }
        let started = Instant::now();
        let budget = self.limits().construction_deadline;
        let handle = match self.launcher().construct(self.key(), id, budget) {
            Ok(handle) => handle,
            Err(fault) => {
                return Err(self.fail_construction(
                    worker,
                    None,
                    None,
                    ConstructionFault::Launcher(fault),
                ));
            }
        };
        let remaining = budget.saturating_sub(started.elapsed());
        let (sterile, refs) = match self.broker().prepare(self.key(), id, remaining) {
            Ok(parts) => parts,
            Err(fault) => {
                return Err(self.fail_construction(
                    worker,
                    Some(handle),
                    None,
                    ConstructionFault::Resources(fault),
                ));
            }
        };
        let elapsed = started.elapsed();
        if elapsed > budget {
            let fault = ConstructionFault::Deadline { budget, elapsed };
            return Err(self.fail_construction(worker, Some(handle), Some(sterile), fault));
        }
        let identity = handle.identity();
        let record = Record::new(RecordKind::Sterile, id, worker.generation(), key)
            .identity(identity)
            .resources(refs);
        if let Err(error) = self.record(&record) {
            return Err(self.fail_construction(
                worker,
                Some(handle),
                Some(sterile),
                ConstructionFault::Ledger(error),
            ));
        }
        self.prepared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                id,
                Prepared {
                    handle,
                    sterile,
                    refs,
                    identity,
                },
            );
        if let Err(race) = worker.sterilize() {
            let prepared = self
                .prepared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&id);
            if let Some(prepared) = prepared {
                let _ = prepared.handle.destroy();
                let _ = self.broker().release_sterile(prepared.sterile);
            }
            return Err(failure(ConstructionFault::State(race)));
        }
        Ok(id)
    }

    fn fail_construction(
        &self,
        worker: Worker<Constructing>,
        handle: Option<L::Handle>,
        sterile: Option<R::Sterile>,
        fault: ConstructionFault,
    ) -> ConstructionFailure {
        let id = worker.id();
        let generation = worker.generation();
        if let Some(handle) = handle {
            let _ = handle.destroy();
        }
        if let Some(sterile) = sterile {
            let _ = self.broker().release_sterile(sterile);
        }
        let _ = self.record(&Record::new(
            RecordKind::ConstructFailed,
            id,
            generation,
            self.digest(),
        ));
        let _ = worker.abort();
        ConstructionFailure {
            worker: Some(id),
            fault,
        }
    }
}
