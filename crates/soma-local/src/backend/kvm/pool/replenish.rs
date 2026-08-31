//! Building sterile machines ahead of demand, on a thread of the pool's own.
//!
//! Nothing here runs on a request path. The whole point of the pool is that the restore has
//! already happened when a Launch arrives, so replenishment is a background loop that converges
//! on the pool's target and then sleeps until something changes.
//!
//! Construction happens outside the pool lock, because building a machine takes milliseconds and
//! a claimer must not wait behind it. The served key is therefore rechecked when the finished
//! machine is admitted: a machine prepared for a key the pool no longer serves is destroyed
//! rather than admitted, since it can never satisfy a request.

use std::sync::atomic::Ordering;
use std::sync::{Arc, MutexGuard};

use soma_hostd::{Constructing, Slot, Worker};

use super::{Entry, IDLE_WAIT, MachineKey, Recipe, Shared, State};
use crate::backend::kvm::session::Session;
use crate::backend::kvm::sterile::SterileSpec;

/// What the loop decided to do next while it held the lock.
enum Next {
    /// Build one machine for this key from this specification.
    Build(Box<SterileSpec>, MachineKey),
    /// There is nothing to build; wait to be woken.
    Wait,
}

/// Keeps the pool at its target until the pool is dropped.
pub(super) fn run(shared: &Arc<Shared>) {
    while !shared.stopped.load(Ordering::Acquire) {
        match decide(shared) {
            Next::Build(spec, key) => build(shared, *spec, &key),
            Next::Wait => wait(shared),
        }
    }
}

/// Decides the next action under the lock, so the count and the recipe are read together.
fn decide(shared: &Arc<Shared>) -> Next {
    let state = shared.lock();
    if state.sterile.len() >= shared.limits.target || state.sterile.len() >= shared.limits.max {
        return Next::Wait;
    }
    let Some(recipe) = state.recipe.as_ref() else {
        return Next::Wait;
    };
    let key = recipe.key().clone();
    recipe
        .spec()
        .map_or(Next::Wait, |spec| Next::Build(Box::new(spec), key))
}

/// Builds one sterile machine and admits it, or destroys whatever it produced.
fn build(shared: &Arc<Shared>, spec: SterileSpec, key: &MachineKey) {
    let Some(id) = shared.fresh_worker_id() else {
        wait(shared);
        return;
    };
    let slot = Slot::new(id, key.digest());
    let constructing = Worker::<Constructing>::open(Arc::clone(&slot));
    if let Ok(session) = Session::prepare(spec) {
        admit(shared, slot, constructing, session, key);
    } else {
        // Nothing was ever claimable, so the slot goes straight to its terminal phase. The
        // bounded wait keeps a host that cannot restore at all from spinning on the failure.
        let _ignored = constructing.abort();
        wait(shared);
    }
}

/// Publishes one finished machine, unless the pool moved on while it was being built.
fn admit(
    shared: &Arc<Shared>,
    slot: Arc<Slot>,
    constructing: Worker<Constructing>,
    session: Session,
    key: &MachineKey,
) {
    let mut state = shared.lock();
    let wanted = state.recipe.as_ref().map(Recipe::key) == Some(key)
        && state.sterile.len() < shared.limits.max
        && !shared.stopped.load(Ordering::Acquire);
    // Sterilizing is what makes the slot claimable, so it happens only once the machine is
    // certainly going into the table. A machine that is not wanted is dropped with its slot
    // still unclaimable, which releases the VM, the mapping, and every descriptor it holds.
    if wanted && constructing.sterilize().is_ok() {
        state.sterile.push(Entry { slot, session });
    } else {
        drop(session);
    }
}

/// Sleeps until the pool is woken or the bounded idle wait expires.
fn wait(shared: &Arc<Shared>) {
    let state = shared.lock();
    if shared.stopped.load(Ordering::Acquire) {
        return;
    }
    let (guard, _timed_out) = shared
        .wake
        .wait_timeout(state, IDLE_WAIT)
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    drop::<MutexGuard<'_, State>>(guard);
}
