use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use crate::{
    InstanceId, StateRecord, StateRevision, StateStore, StateStoreFailure, StateStoreFailureKind,
    StoredState,
};

/// A clone-shareable, process-local state store for tests and ephemeral development only.
#[derive(Clone, Default)]
pub struct MemoryStateStore {
    records: Arc<Mutex<BTreeMap<InstanceId, StoredState>>>,
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
}

const fn conflict() -> StateStoreFailure {
    StateStoreFailure::new(StateStoreFailureKind::Conflict)
}

const fn unavailable() -> StateStoreFailure {
    StateStoreFailure::new(StateStoreFailureKind::Unavailable)
}
