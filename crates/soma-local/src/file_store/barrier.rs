//! Group commit for the state store's durable writes.
//!
//! A create makes two durable writes (`Engine::launch_machine`'s write-ahead
//! intent and its post-ready publication), and each write is a sync of the
//! record's bytes and a sync of the link that publishes them. On a disk-backed
//! state root that is four syncs per create, and they serialize: the disk
//! commits one at a time, so the cost grows with concurrency instead of staying
//! flat. Measured on host-04 with the state root on disk, the two writes were
//! 8.1 ms of an 11.3 ms create at c=1 and 33.2 ms of 40.8 ms at c=33, with the
//! p90 of the first write reaching 40 ms.
//!
//! One barrier serves every writer on a state root. A writer calls
//! [`SyncBarrier::commit`] once its bytes are written and once its link is
//! published; the first writer to arrive performs the sync for every writer
//! that arrived with it, and the rest are released by that one call. On Linux
//! the sync is `syncfs`, which commits everything already dirty on the
//! filesystem, so a single call covers the whole batch.
//!
//! The guarantee is the one the store already had, and it is why there is no
//! loss window: `commit` returns only after a sync that provably covers the
//! caller's own write. A writer is counted into a batch only if it arrived
//! before the leader took its snapshot, and the sync runs after that snapshot,
//! so every writer released with `Ok` had its write flushed by the completed
//! sync. A writer that arrives while a sync is running waits for the next one
//! rather than being released by a sync that may have started before its write.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use soma::StateStoreFailure;

/// How long the leader of a batch lets other writers join it before it syncs.
///
/// The cost is paid once per batch and is bounded well below the disk sync it
/// replaces, so on a disk-backed root it is a few percent of the call. It is
/// what makes concurrent writers share a sync instead of each paying their own;
/// without it a writer that arrives while the leader already holds the
/// leadership would wait for the following sync, and a busy root would pay one
/// sync per writer again.
const BATCH_WINDOW: Duration = Duration::from_micros(200);

/// One barrier per state root, shared by every store opened on that root.
///
/// The store is opened once per facade and a runner holds a pool of facades in
/// one process, so a barrier owned by a single store would never see two
/// writers at once. This registry is what makes a batch a batch. The entry
/// lives for the process; a state root is a deployment constant, so the map
/// holds one entry per root in practice.
static BARRIERS: Mutex<BTreeMap<PathBuf, Arc<SyncBarrier>>> = Mutex::new(BTreeMap::new());

/// Returns the barrier for `root`, creating it on first use.
pub(super) fn for_root(root: &Path) -> Arc<SyncBarrier> {
    let mut barriers = BARRIERS.lock().unwrap_or_else(PoisonError::into_inner);
    Arc::clone(
        barriers
            .entry(root.to_path_buf())
            .or_insert_with(|| Arc::new(SyncBarrier::new(BATCH_WINDOW))),
    )
}

/// One batch of writes that share a sync.
pub(super) struct SyncBarrier {
    batch: Mutex<Batch>,
    released: Condvar,
    window: Duration,
}

#[derive(Default)]
struct Batch {
    /// Writers whose own file work is complete and who are waiting to be made
    /// durable.
    arrived: u64,
    /// The highest `arrived` a completed sync provably covers.
    synced: u64,
    /// One writer performs the sync; every other writer waits for it.
    syncing: bool,
    /// What the in-flight sync reported, for the writers it releases.
    failure: Option<StateStoreFailure>,
}

impl SyncBarrier {
    pub(super) fn new(window: Duration) -> Self {
        Self {
            batch: Mutex::new(Batch::default()),
            released: Condvar::new(),
            window,
        }
    }

    /// Runs `sync` once for this writer and for every writer that joined it, and
    /// returns only once this writer's own write is durable.
    ///
    /// # Errors
    ///
    /// Returns the failure the sync reported. A writer that waited for another
    /// writer's sync receives that sync's failure rather than a success it
    /// cannot vouch for.
    pub(super) fn commit(
        &self,
        sync: impl FnOnce() -> Result<(), StateStoreFailure>,
    ) -> Result<(), StateStoreFailure> {
        let mut batch = self.batch.lock().unwrap_or_else(PoisonError::into_inner);
        let mine = batch.arrived;
        batch.arrived += 1;
        if batch.syncing {
            return self.wait(batch, mine);
        }
        batch.syncing = true;
        batch.failure = None;
        drop(batch);

        // Writers already blocked on the mutex join this batch; anyone slower
        // than the window waits for the next sync instead of being released by
        // one that may have started before its write.
        std::thread::sleep(self.window);
        let covered = {
            let batch = self.batch.lock().unwrap_or_else(PoisonError::into_inner);
            batch.arrived
        };

        let result = sync();

        let mut batch = self.batch.lock().unwrap_or_else(PoisonError::into_inner);
        match result {
            Ok(()) => batch.synced = covered,
            Err(failure) => batch.failure = Some(failure),
        }
        batch.syncing = false;
        drop(batch);
        self.released.notify_all();
        result
    }

    /// Waits until a sync that started after this writer arrived completes.
    fn wait(&self, mut batch: MutexGuard<'_, Batch>, mine: u64) -> Result<(), StateStoreFailure> {
        loop {
            if batch.synced > mine {
                return Ok(());
            }
            if let Some(failure) = batch.failure {
                return Err(failure);
            }
            batch = self
                .released
                .wait(batch)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

#[cfg(test)]
#[path = "barrier_tests.rs"]
mod tests;
