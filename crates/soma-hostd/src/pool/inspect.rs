//! Read-only views of the pool: occupancy per phase and one worker's state.

use std::sync::Arc;

use crate::{Occupancy, Phase, Pool, ResourceBroker, Slot, WorkerId, WorkerLauncher, WorkerView};

impl<L: WorkerLauncher, R: ResourceBroker> Pool<L, R> {
    /// Counts workers per phase.
    #[must_use]
    pub fn occupancy(&self) -> Occupancy {
        let mut occupancy = Occupancy::default();
        for slot in self
            .slots
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
        {
            match slot.observe().phase {
                Phase::Constructing => occupancy.constructing += 1,
                Phase::Sterile => occupancy.sterile += 1,
                Phase::Claiming => occupancy.claiming += 1,
                Phase::Assigned => occupancy.assigned += 1,
                Phase::Running => occupancy.running += 1,
                Phase::Destroying => occupancy.destroying += 1,
                Phase::Dead => occupancy.dead += 1,
            }
        }
        occupancy
    }

    /// Reports one worker.
    #[must_use]
    pub fn inspect(&self, worker: WorkerId) -> Option<WorkerView> {
        let slot = self.find_slot(worker)?;
        let observed = slot.observe();
        let owned = self
            .owned
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (operation, instance, launch) =
            owned.get(&worker).map_or((None, None, None), |owned| {
                (Some(owned.operation), Some(owned.instance), owned.launch)
            });
        Some(WorkerView {
            worker,
            phase: observed.phase,
            lease_generation: observed.generation,
            operation,
            instance,
            launch,
        })
    }

    pub(crate) fn find_slot(&self, worker: WorkerId) -> Option<Arc<Slot>> {
        self.slots
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find(|slot| slot.id() == worker)
            .cloned()
    }

    pub(crate) fn slots(&self) -> Vec<Arc<Slot>> {
        self.slots
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}
