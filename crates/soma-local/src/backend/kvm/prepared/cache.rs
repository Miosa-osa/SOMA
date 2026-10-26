//! The admitted Generations this process holds, and how one entry leaves them.
//!
//! An admitted Generation keeps its verified artifacts open, so the cache is what holds a host's
//! prepared bytes open. Releasing an entry is therefore a cache operation: the value leaves the
//! cache, and the entry leaves the root, under one lock, so no lookup can find the value gone and
//! the entry still present, re-admit it, and reopen what was just released.

use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};

use super::{CacheKey, PreparedGeneration};

/// A cached value that can say whether anything outside the cache still holds it.
pub(super) trait Retained {
    /// Whether a holder other than the cache keeps this value's open handles alive.
    fn held_elsewhere(&self) -> bool;
}

impl Retained for PreparedGeneration {
    /// Every clone shares the artifact handles, so a count above one is a clone some launch,
    /// prepared machine, or prewarm plan still holds.
    fn held_elsewhere(&self) -> bool {
        self.artifacts
            .iter()
            .any(|(_, file)| Arc::strong_count(file) > 1)
    }
}

/// Admitted values by prepared root and reference.
pub(super) struct Cache<T> {
    map: Mutex<HashMap<CacheKey, T>>,
}

/// What releasing one entry did.
pub(super) enum Release<T> {
    /// The entry left the root, and this is the value that left the cache, if one was there.
    Moved(Option<T>),
    /// Something outside the cache holds the value, so nothing moved.
    Held,
    /// The entry could not be moved, so the value stayed cached.
    Failed(std::io::Error),
}

impl<T: Clone + Retained> Cache<T> {
    pub(super) fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<CacheKey, T>> {
        self.map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn get(&self, key: &CacheKey) -> Option<T> {
        self.lock().get(key).cloned()
    }

    /// Whether the cached value for `key` is held outside the cache. Changes nothing.
    pub(super) fn held_elsewhere(&self, key: &CacheKey) -> bool {
        self.lock().get(key).is_some_and(Retained::held_elsewhere)
    }

    /// Caches `value` unless its entry left the root after it was read.
    ///
    /// Release moves the entry under this same lock, so either the move came first and this
    /// refuses, or this came first and release sees the value and the caller's clone of it.
    pub(super) fn admit(&self, key: CacheKey, value: T, entry: &Path) -> bool {
        let mut map = self.lock();
        if std::fs::symlink_metadata(entry).is_err() {
            return false;
        }
        map.insert(key, value);
        true
    }

    /// Takes `key` out of the cache and moves `from` to `to`, or does neither.
    pub(super) fn release(&self, key: &CacheKey, from: &Path, to: &Path) -> Release<T> {
        let mut map = self.lock();
        if map.get(key).is_some_and(Retained::held_elsewhere) {
            return Release::Held;
        }
        match std::fs::rename(from, to) {
            Ok(()) => Release::Moved(map.remove(key)),
            Err(error) => Release::Failed(error),
        }
    }

    /// Moves `to` back to `from` and caches `value` again, undoing [`Self::release`].
    pub(super) fn restore(
        &self,
        key: CacheKey,
        value: Option<T>,
        from: &Path,
        to: &Path,
    ) -> std::io::Result<()> {
        let mut map = self.lock();
        std::fs::rename(to, from)?;
        if let Some(value) = value {
            map.insert(key, value);
        }
        Ok(())
    }
}

/// The cache every lookup in this process shares.
pub(super) fn generations() -> &'static Cache<PreparedGeneration> {
    static CACHE: OnceLock<Cache<PreparedGeneration>> = OnceLock::new();
    CACHE.get_or_init(Cache::new)
}
