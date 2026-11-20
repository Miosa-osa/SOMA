//! How the broker's background work keeps out of the request path's way.
//!
//! Building and tearing down bundles are background work: no request waits for them, but they
//! contend with claims and activations for the same kernel locks. [`Traffic`] lets that work
//! wait until the request path is quiet, and [`Permits`] bounds how many teardowns run at once.

use std::{
    sync::{
        Condvar, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

/// How long the request path must have been idle before background work starts.
const QUIET_MILLIS: u64 = 150;

/// Claims and activations in flight, and when the last one finished.
///
/// Building and tearing down bundles contend with a claim or an activation for the kernel's
/// network configuration lock, and an activation that waits behind a hundred background builds
/// is a sandbox that is slow to become reachable. Background work therefore waits for the
/// request path to go quiet.
pub(super) struct Traffic {
    inflight: AtomicUsize,
    last_end_millis: AtomicU64,
    epoch: Instant,
}

impl Traffic {
    pub(super) fn new() -> Self {
        Self {
            inflight: AtomicUsize::new(0),
            last_end_millis: AtomicU64::new(0),
            epoch: Instant::now(),
        }
    }

    fn now_millis(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Marks one request-path operation in flight until the returned guard drops.
    pub(super) fn enter(&self) -> InFlight<'_> {
        self.inflight.fetch_add(1, Ordering::AcqRel);
        InFlight(self)
    }

    fn quiet(&self) -> bool {
        self.inflight.load(Ordering::Acquire) == 0
            && self
                .now_millis()
                .saturating_sub(self.last_end_millis.load(Ordering::Acquire))
                >= QUIET_MILLIS
    }

    /// Waits until the request path has been idle for the quiet interval, or `patience` ends.
    pub(super) fn wait_quiet(&self, patience: Duration) {
        let deadline = Instant::now() + patience;
        while !self.quiet() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// A counting limit on how many teardowns run at once.
///
/// Each teardown spawns privileged tools that wait on kernel locks, and a hundred spawned at
/// once do not finish sooner than eight at a time; they miss the tool deadline instead, which
/// turns a slow release into an incomplete one.
pub(super) struct Permits {
    free: Mutex<usize>,
    freed: Condvar,
}

impl Permits {
    pub(super) const fn new(count: usize) -> Self {
        Self {
            free: Mutex::new(count),
            freed: Condvar::new(),
        }
    }

    pub(super) fn acquire(&self) -> Permit<'_> {
        let mut free = self
            .free
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *free == 0 {
            free = self
                .freed
                .wait(free)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *free -= 1;
        Permit(self)
    }
}

pub(super) struct Permit<'a>(&'a Permits);

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        *self
            .0
            .free
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        self.0.freed.notify_one();
    }
}

pub(super) struct InFlight<'a>(&'a Traffic);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0
            .last_end_millis
            .store(self.0.now_millis(), Ordering::Release);
        self.0.inflight.fetch_sub(1, Ordering::AcqRel);
    }
}
