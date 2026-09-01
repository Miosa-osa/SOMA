mod enumerate;
mod failure;
mod filesystem;
mod layout;
mod lock;
mod revision;

use std::{
    fmt,
    path::{Path, PathBuf},
};

use soma::{InstanceId, StateRecord, StateRevision, StateStore, StateStoreFailure, StoredState};

use self::{
    enumerate::instance_identities,
    failure::{capacity_exceeded, conflict, invalid_record, unavailable},
    filesystem::{ensure_directory, existing_directory},
    layout::{LOCK_DIRECTORY, shard_lock_path},
    lock::open_lock,
    revision::{commit_revision, prune_superseded, read_record, scan_revisions},
};

/// A process-safe, revisioned local store rooted at an explicit caller-owned directory.
pub struct FileStateStore {
    root: PathBuf,
    locks: PathBuf,
}

impl FileStateStore {
    /// Opens or creates an owner-private state root.
    ///
    /// # Errors
    ///
    /// Returns a typed storage failure when the root is empty, unsafe, or unavailable.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StateStoreFailure> {
        let root = root.into();
        if root.as_os_str().is_empty() {
            return Err(invalid_record());
        }
        ensure_directory(&root)?;
        let locks = root.join(LOCK_DIRECTORY);
        ensure_directory(&locks)?;
        Ok(Self { root, locks })
    }

    fn with_instance_lock<T>(
        &self,
        instance_id: &InstanceId,
        operation: impl FnOnce(&Path) -> Result<T, StateStoreFailure>,
    ) -> Result<T, StateStoreFailure> {
        let directory = self.root.join(instance_id.as_str());
        let lock = open_lock(&shard_lock_path(&self.locks, instance_id))?;
        lock.lock().map_err(|_| unavailable())?;
        let result = operation(&directory);
        let unlocked = lock.unlock().map_err(|_| unavailable());
        match (result, unlocked) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) | (Ok(_), Err(error)) => Err(error),
        }
    }
}

impl fmt::Debug for FileStateStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FileStateStore")
    }
}

impl StateStore for FileStateStore {
    fn create(
        &mut self,
        instance_id: &InstanceId,
        record: StateRecord,
    ) -> Result<StateRevision, StateStoreFailure> {
        self.with_instance_lock(instance_id, |directory| {
            ensure_directory(directory)?;
            if !scan_revisions(directory)?.is_empty() {
                return Err(conflict());
            }
            commit_revision(directory, StateRevision::INITIAL, &record)?;
            Ok(StateRevision::INITIAL)
        })
    }

    fn load(&mut self, instance_id: &InstanceId) -> Result<Option<StoredState>, StateStoreFailure> {
        self.with_instance_lock(instance_id, |directory| {
            if !existing_directory(directory)? {
                return Ok(None);
            }
            let revisions = scan_revisions(directory)?;
            let Some((revision, path)) = revisions.last_key_value() else {
                return Ok(None);
            };
            let record = read_record(path)?;
            Ok(Some(StoredState::new(*revision, record)))
        })
    }

    fn compare_exchange(
        &mut self,
        instance_id: &InstanceId,
        expected: StateRevision,
        replacement: StateRecord,
    ) -> Result<StateRevision, StateStoreFailure> {
        self.with_instance_lock(instance_id, |directory| {
            if !existing_directory(directory)? {
                return Err(conflict());
            }
            let revisions = scan_revisions(directory)?;
            let Some((current, current_path)) = revisions.last_key_value() else {
                return Err(conflict());
            };
            if *current != expected {
                return Err(conflict());
            }
            read_record(current_path)?;
            let next_value = expected
                .get()
                .checked_add(1)
                .ok_or_else(capacity_exceeded)?;
            let next = StateRevision::new(next_value).map_err(|_| capacity_exceeded())?;
            commit_revision(directory, next, &replacement)?;
            prune_superseded(&revisions, next);
            Ok(next)
        })
    }

    fn list(&mut self) -> Result<Vec<InstanceId>, StateStoreFailure> {
        instance_identities(&self.root)
    }
}

#[cfg(test)]
mod tests;
