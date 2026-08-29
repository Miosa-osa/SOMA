//! Background construction threads bounded by the replenishment concurrency.
//!
//! Every pass reaps the threads that have finished before it starts more, so a long-lived
//! daemon retains one join handle per construction still running and never one per
//! construction ever started.

use std::{
    sync::{Arc, atomic::Ordering},
    thread,
};

use super::{ReplenishLimit, ReplenishReport};
use crate::{Pool, ResourceBroker, WorkerLauncher};

impl<L: WorkerLauncher, R: ResourceBroker> Pool<L, R> {
    /// Starts bounded background constructions toward the target and returns at once.
    #[must_use]
    pub fn replenish(self: &Arc<Self>) -> ReplenishReport {
        if self.needs_reconcile() {
            return ReplenishReport {
                deficit: self.limits().target,
                spawned: 0,
                in_flight: self.in_flight.load(Ordering::Acquire),
                limited_by: Some(ReplenishLimit::Unreconciled),
            };
        }
        self.reap_replenishment();
        let mut spawned = 0;
        let mut limited_by = None;
        let gate = self
            .replenish_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            let occupancy = self.occupancy();
            let in_flight = self.in_flight.load(Ordering::Acquire);
            let deficit = self
                .limits()
                .target
                .saturating_sub(occupancy.sterile + in_flight);
            if deficit == 0 {
                break;
            }
            if in_flight >= self.limits().replenish_concurrency {
                limited_by = Some(ReplenishLimit::Concurrency);
                break;
            }
            if occupancy.live() - occupancy.constructing + in_flight >= self.limits().max {
                limited_by = Some(ReplenishLimit::PoolMaximum);
                break;
            }
            self.in_flight.fetch_add(1, Ordering::AcqRel);
            let pool = Arc::clone(self);
            let handle = thread::spawn(move || {
                let _ = pool.construct_reserved();
                pool.in_flight.fetch_sub(1, Ordering::AcqRel);
            });
            self.threads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(handle);
            spawned += 1;
        }
        drop(gate);
        self.reap_replenishment();
        ReplenishReport {
            deficit: self
                .limits()
                .target
                .saturating_sub(self.occupancy().sterile + self.in_flight.load(Ordering::Acquire)),
            spawned,
            in_flight: self.in_flight.load(Ordering::Acquire),
            limited_by,
        }
    }

    /// Construction threads this pool has started and not yet joined.
    #[must_use]
    pub fn pending_replenishment(&self) -> usize {
        self.threads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Joins every construction thread that has already finished.
    pub fn reap_replenishment(&self) {
        let finished: Vec<_> = {
            let mut threads = self
                .threads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut finished = Vec::new();
            let mut index = 0;
            while index < threads.len() {
                if threads[index].is_finished() {
                    finished.push(threads.swap_remove(index));
                } else {
                    index += 1;
                }
            }
            finished
        };
        for handle in finished {
            let _ = handle.join();
        }
    }

    /// Joins every construction thread started by [`Pool::replenish`].
    pub fn wait_replenishment(&self) {
        let handles: Vec<_> = self
            .threads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(..)
            .collect();
        for handle in handles {
            let _ = handle.join();
        }
    }
}
