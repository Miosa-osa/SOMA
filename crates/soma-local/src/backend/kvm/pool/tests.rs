//! What the pool must guarantee without a kernel: a single winner, no way back into the pool,
//! and an honest answer when nothing is prepared.
//!
//! Restoring a real machine needs `/dev/kvm`, the pinned kernel, and a captured Generation, so
//! these exercise the slot table and the claim rather than the machine behind it. That a claimed
//! machine reaches Ready, and what it costs, is proved live in the `soma-kvm` restore suite.

use std::sync::Arc;

use soma_hostd::{Claiming, Constructing, Phase, Slot, Worker, WorkerId};

use super::{MachineKey, MachinePool, destroy};
use crate::backend::kvm::limits;

/// Any key; the pool only ever compares one for equality and digests it.
fn key() -> MachineKey {
    MachineKey {
        candidate: [3; 32],
        snapshot: "/srv/generations/one/snapshot".into(),
        memory_bytes: 1 << 30,
        overlay_capacity_bytes: 256 << 20,
        vcpus: 1,
    }
}

/// One sterile slot, ready to be raced for.
fn sterile_slot(id: [u8; 16]) -> Arc<Slot> {
    let slot = Slot::new(
        WorkerId::new(id).expect("nonzero worker identity"),
        key().digest(),
    );
    Worker::<Constructing>::open(Arc::clone(&slot))
        .sterilize()
        .expect("a fresh slot sterilizes");
    slot
}

/// Ownership is one compare-and-swap, so a hundred threads racing one sterile machine produce
/// exactly one winner and ninety-nine callers who correctly see nothing to claim.
#[test]
fn exactly_one_claimer_wins_one_sterile_machine() {
    let slot = sterile_slot([9; 16]);

    let winners: usize = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..100)
            .map(|_| {
                let slot = Arc::clone(&slot);
                scope.spawn(move || usize::from(slot.try_claim().is_some()))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or(0))
            .sum()
    });

    assert_eq!(winners, 1, "more than one claimer won one sterile machine");
    assert_eq!(slot.observe().phase, Phase::Claiming);
}

/// A claim that does not complete its transfer destroys the worker. The phase table has no edge
/// back to sterile, so an abandoned claim cannot be answered by putting the machine back.
#[test]
fn an_abandoned_claim_never_returns_the_machine_to_the_pool() {
    let slot = sterile_slot([4; 16]);
    let worker: Worker<Claiming> = slot.try_claim().expect("the first claimer wins");

    destroy(worker);

    assert_eq!(slot.observe().phase, Phase::Dead);
    assert!(
        slot.try_claim().is_none(),
        "a destroyed worker was offered to another claimer"
    );
}

/// A machine already claimed by somebody else cannot be evicted out from under them, which is
/// what lets the pool retarget its key while a Launch is mid-transfer.
#[test]
fn eviction_loses_to_a_claim_that_already_won() {
    let slot = sterile_slot([5; 16]);
    let _claimed = slot.try_claim().expect("the claimer wins");

    assert!(
        slot.try_evict().is_none(),
        "an eviction took a machine a claimer already owned"
    );
}

/// A pool that has prepared nothing says so. It never answers a claim with a machine, which is
/// what keeps a depleted pool from being reported as a prepared launch.
#[test]
fn a_pool_with_nothing_prepared_reports_no_machine() {
    let pool = MachinePool::open(limits(0)).expect("a pool that prepares nothing opens");

    assert!(pool.claim(&key()).is_none());
    assert_eq!(pool.sterile_count(), 0);
}
