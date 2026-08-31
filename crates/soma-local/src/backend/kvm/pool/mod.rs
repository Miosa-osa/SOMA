//! A bounded pool of sterile machines, claimed on Launch.
//!
//! Restoring a machine costs the same work whichever Instance it ends up serving, so this pool
//! pays that work before a request arrives and a Launch claims the result. Ownership is decided
//! by the one compare-and-swap in `soma-hostd`'s [`Slot::try_claim`], which is the single-winner
//! claim of the prepared worker protocol: exactly one caller may move a slot out of `Sterile`,
//! and the slot's lease generation is bumped by that move so a loser can never act on it.
//!
//! A claim that does not certainly complete its transfer destroys the worker. Nothing here ever
//! returns a machine to the pool once it has been claimed, because a machine that saw one
//! Instance's authority cannot be shown to hold none of it.
//!
//! An empty pool is not an error and is never disguised as a hit: [`MachinePool::claim`] returns
//! `None` and the Launch takes its own on-demand path, which it reports as on-demand.

mod key;
mod replenish;
#[cfg(test)]
mod tests;

use std::sync::{
    Arc, Condvar, Mutex, MutexGuard, PoisonError,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::thread::JoinHandle;
use std::time::Duration;

use soma_hostd::{Claiming, Limits, LimitsError, Phase, Slot, Worker, WorkerId};

pub(super) use key::{MachineKey, Recipe};

use super::session::{Session, SessionError};
use super::sterile::Assignment;

/// How long the replenisher sleeps when there is nothing to build or a build just failed.
///
/// The wait is bounded rather than indefinite so that a pool whose wake-up was missed still
/// converges on its target instead of staying empty until the next request.
const IDLE_WAIT: Duration = Duration::from_secs(1);

/// One sterile machine and the slot that decides who owns it.
struct Entry {
    slot: Arc<Slot>,
    session: Session,
}

/// Everything the replenisher and the claimers share.
struct Shared {
    limits: Limits,
    state: Mutex<State>,
    wake: Condvar,
    stopped: AtomicBool,
    counter: AtomicU64,
}

/// What the pool holds, under one lock.
#[derive(Default)]
struct State {
    /// The one key this pool prepares for, and how to build another machine for it.
    recipe: Option<Recipe>,
    /// The sterile machines, each with its slot.
    sterile: Vec<Entry>,
}

/// A bounded pool of sterile machines for one key at a time.
pub(in crate::backend) struct MachinePool {
    shared: Arc<Shared>,
    replenisher: Option<JoinHandle<()>>,
}

impl MachinePool {
    /// Opens a pool under `limits` and starts replenishing outside every request path.
    ///
    /// A target of zero opens a pool that never prepares anything, so every Launch takes the
    /// on-demand path and says so. That is the configuration an operator uses to turn
    /// preparation off without changing what a Launch reports.
    ///
    /// # Errors
    ///
    /// Returns the violated limits rule.
    pub(in crate::backend) fn open(limits: Limits) -> Result<Self, LimitsError> {
        let shared = Arc::new(Shared {
            limits: limits.validate()?,
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            stopped: AtomicBool::new(false),
            counter: AtomicU64::new(0),
        });
        let replenisher = (limits.target > 0)
            .then(|| {
                let worker = Arc::clone(&shared);
                std::thread::Builder::new()
                    .name("soma-kvm-replenish".to_owned())
                    .spawn(move || replenish::run(&worker))
                    .ok()
            })
            .flatten();
        Ok(Self {
            shared,
            replenisher,
        })
    }

    /// Names the key this pool prepares for from now on.
    ///
    /// Registering a different key evicts every machine prepared for the previous one: a sterile
    /// machine was built against exactly one snapshot, shape, and head size, and cannot be
    /// retargeted. Eviction goes through the same compare-and-swap a claim uses, so a machine
    /// another thread is claiming at that instant stays with the claimer.
    pub(in crate::backend) fn serve(&self, recipe: Recipe) {
        let mut state = self.shared.lock();
        if state.recipe.as_ref().map(Recipe::key) != Some(recipe.key()) {
            for entry in std::mem::take(&mut state.sterile) {
                // A machine whose slot was already claimed belongs to its claimer, so only the
                // ones this eviction wins are dropped here.
                if entry.slot.try_evict().is_some() {
                    drop(entry.session);
                }
            }
        }
        state.recipe = Some(recipe);
        drop(state);
        self.shared.wake.notify_all();
    }

    /// Claims one sterile machine prepared for `key`, or reports that none exists.
    ///
    /// `None` means the pool is empty or holds machines for another key. It is never a failure
    /// and never a prepared launch: the caller falls back to its own on-demand path.
    pub(in crate::backend) fn claim(&self, key: &MachineKey) -> Option<Claimed> {
        let mut state = self.shared.lock();
        if state.recipe.as_ref().map(Recipe::key) != Some(key) {
            return None;
        }
        // Exactly one caller can win each slot. Losing one is not a reason to give up on the
        // pool, so the scan continues to the next machine rather than falling back with sterile
        // machines still sitting in the table.
        let won = state.sterile.iter().position(|entry| {
            entry.slot.observe().phase == Phase::Sterile && entry.slot.try_claim().is_some()
        })?;
        let entry = state.sterile.remove(won);
        drop(state);
        self.shared.wake.notify_all();
        Worker::<Claiming>::attach(entry.slot).map(|worker| Claimed {
            worker: Some(worker),
            session: Some(entry.session),
        })
    }

    /// How many sterile machines the pool holds right now.
    #[cfg(test)]
    pub(in crate::backend) fn sterile_count(&self) -> usize {
        self.shared.lock().sterile.len()
    }
}

impl Drop for MachinePool {
    /// Stops replenishing and destroys every machine the pool still holds.
    ///
    /// The replenisher is joined rather than detached so that a machine still being built is
    /// released before the process moves on, instead of outliving the pool that owns it.
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        self.shared.wake.notify_all();
        if let Some(thread) = self.replenisher.take() {
            let _ignored = thread.join();
        }
        let mut state = self.shared.lock();
        state.recipe = None;
        state.sterile.clear();
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A worker identity that is unique within this process and never all zero.
    ///
    /// The pool has no durable ledger, so the identity only has to distinguish the workers this
    /// process holds; the process number is mixed in so a diagnostic naming one worker cannot be
    /// confused with another process's.
    fn fresh_worker_id(&self) -> Option<WorkerId> {
        let counter = self.counter.fetch_add(1, Ordering::Relaxed);
        let mut bytes = [0_u8; 16];
        bytes[..8].copy_from_slice(&counter.to_be_bytes());
        bytes[8..12].copy_from_slice(&std::process::id().to_be_bytes());
        // The last byte is set so the identity is never the all-zero value, which is refused.
        bytes[15] = 1;
        WorkerId::new(bytes).ok()
    }
}

/// One machine whose slot this caller won, and which it must assign or destroy.
///
/// Dropping it without a completed assignment destroys the machine. That is the whole rule of
/// the transfer: a worker leaves the pool once, and it either receives one Instance's authority
/// or it is gone.
pub(in crate::backend) struct Claimed {
    worker: Option<Worker<Claiming>>,
    session: Option<Session>,
}

impl Claimed {
    /// Transfers fresh Instance authority into the claimed machine.
    ///
    /// # Errors
    ///
    /// Returns the session failure; the machine is destroyed rather than reused.
    pub(in crate::backend) fn assign(
        mut self,
        assignment: Assignment,
    ) -> Result<Session, SessionError> {
        let (Some(worker), Some(mut session)) = (self.worker.take(), self.session.take()) else {
            return Err(SessionError::Poisoned);
        };
        match session.assign(assignment) {
            Ok(()) => {
                // The slot follows the machine: an assignment that reached Ready is a running
                // Instance, and the phase table forbids either phase from returning to sterile.
                let running = worker.assign().and_then(Worker::run);
                debug_assert!(running.is_ok(), "a claimed slot moved without its claimer");
                Ok(session)
            }
            Err(error) => {
                destroy(worker);
                drop(session);
                Err(error)
            }
        }
    }
}

impl Drop for Claimed {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            destroy(worker);
        }
    }
}

/// Walks a claimed slot to its terminal phase, which is the only way out of a claim.
fn destroy(worker: Worker<Claiming>) {
    let _ignored = worker.destroy().and_then(Worker::finish);
}
