use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Condvar, Mutex},
};

use crate::{
    InstanceId, InstanceTurn, StateRecord, StateRevision, StateStore, StateStoreFailure,
    StateStoreFailureKind, StoredState,
};

/// A clone-shareable, process-local state store for tests and ephemeral development only.
#[derive(Clone, Default)]
pub struct MemoryStateStore {
    records: Arc<Mutex<BTreeMap<InstanceId, StoredState>>>,
    turns: Arc<Turns>,
}

/// The Instances someone holds the turn for, and the signal that one was given back.
#[derive(Default)]
struct Turns {
    held: Mutex<BTreeSet<InstanceId>>,
    returned: Condvar,
}

struct HeldTurn {
    turns: Arc<Turns>,
    instance_id: InstanceId,
}

impl Drop for HeldTurn {
    fn drop(&mut self) {
        let mut held = self
            .turns
            .held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        held.remove(&self.instance_id);
        drop(held);
        self.turns.returned.notify_all();
    }
}

impl MemoryStateStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl StateStore for MemoryStateStore {
    fn create(
        &mut self,
        instance_id: &InstanceId,
        record: StateRecord,
    ) -> Result<StateRevision, StateStoreFailure> {
        let mut records = self.records.lock().map_err(|_| unavailable())?;
        if records.contains_key(instance_id) {
            return Err(conflict());
        }
        records.insert(
            instance_id.clone(),
            StoredState::new(StateRevision::INITIAL, record),
        );
        Ok(StateRevision::INITIAL)
    }

    fn load(&mut self, instance_id: &InstanceId) -> Result<Option<StoredState>, StateStoreFailure> {
        self.records
            .lock()
            .map_err(|_| unavailable())
            .map(|records| records.get(instance_id).cloned())
    }

    fn compare_exchange(
        &mut self,
        instance_id: &InstanceId,
        expected: StateRevision,
        replacement: StateRecord,
    ) -> Result<StateRevision, StateStoreFailure> {
        let mut records = self.records.lock().map_err(|_| unavailable())?;
        let current = records.get_mut(instance_id).ok_or_else(conflict)?;
        if current.revision() != expected {
            return Err(conflict());
        }
        let revision = expected.next()?;
        *current = StoredState::new(revision, replacement);
        Ok(revision)
    }

    fn release(
        &mut self,
        instance_id: &InstanceId,
        expected: StateRevision,
    ) -> Result<(), StateStoreFailure> {
        let mut records = self.records.lock().map_err(|_| unavailable())?;
        match records.get(instance_id) {
            Some(current) if current.revision() == expected => {
                records.remove(instance_id);
                Ok(())
            }
            _ => Err(conflict()),
        }
    }

    fn list(&mut self) -> Result<Vec<InstanceId>, StateStoreFailure> {
        self.records
            .lock()
            .map_err(|_| unavailable())
            .map(|records| records.keys().cloned().collect())
    }

    fn turn(&self, instance_id: &InstanceId) -> Result<InstanceTurn, StateStoreFailure> {
        let held = self.turns.held.lock().map_err(|_| unavailable())?;
        let mut held = self
            .turns
            .returned
            .wait_while(held, |held| held.contains(instance_id))
            .map_err(|_| unavailable())?;
        held.insert(instance_id.clone());
        Ok(InstanceTurn::new(HeldTurn {
            turns: Arc::clone(&self.turns),
            instance_id: instance_id.clone(),
        }))
    }
}

const fn conflict() -> StateStoreFailure {
    StateStoreFailure::new(StateStoreFailureKind::Conflict)
}

const fn unavailable() -> StateStoreFailure {
    StateStoreFailure::new(StateStoreFailureKind::Unavailable)
}
