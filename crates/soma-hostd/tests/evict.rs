//! Generation eviction: retiring a Generation destroys every sterile worker of its pool.

#![cfg(unix)]

mod support;

use soma_hostd::{ClaimError, DestroyReason, Phase, RecordKind};
use support::{harness, intent, limits, op};

#[test]
fn evicting_a_generation_destroys_every_sterile_worker_and_leaves_the_pool_claimable() {
    let harness = harness(limits(3, 3));
    harness.pool.replenish_blocking().expect("replenish");
    assert_eq!(harness.pool.occupancy().sterile, 3);
    assert_eq!(harness.pool.broker().live_bundles(), 3);
    assert_eq!(harness.table.alive(), 3);
    let before = harness.pool.broker().counters();

    let evidence = harness.pool.evict_sterile();
    assert_eq!(evidence.len(), 3, "every sterile worker was evicted");
    for evicted in &evidence {
        assert_eq!(evicted.reason, DestroyReason::Evicted);
        assert!(evicted.destroyed.complete);
        assert!(evicted.released.complete);
        assert!(evicted.ledger, "both ledger records were written");
        assert_eq!(
            harness.pool.inspect(evicted.worker).map(|view| view.phase),
            Some(Phase::Dead)
        );
    }
    assert_eq!(harness.pool.occupancy().sterile, 0);
    assert_eq!(
        harness.pool.broker().counters().released_sterile,
        before.released_sterile + 3
    );
    assert_eq!(harness.pool.broker().live_bundles(), 0);
    assert_eq!(harness.pool.broker().leased_heads(), 0);
    assert_eq!(
        harness.table.alive(),
        0,
        "every worker process was torn down"
    );

    let records = harness.pool.ledger().records().expect("records");
    let evicted_records = records
        .iter()
        .filter(|(_, record)| {
            record.kind == RecordKind::Destroying && record.detail == DestroyReason::Evicted as u8
        })
        .count();
    assert_eq!(evicted_records, 3);
    let entries = harness.pool.ledger().entries().expect("entries");
    for evicted in &evidence {
        let entry = &entries[&evicted.worker];
        assert_eq!(entry.phase, Phase::Dead);
        assert!(!entry.was_assigned, "an evicted worker was never assigned");
    }

    assert!(
        matches!(
            harness.pool.claim(op(1), intent(1).fingerprint()),
            Err(ClaimError::Exhausted(_))
        ),
        "an evicted pool is exhausted, not empty of policy"
    );
    assert_eq!(
        harness.pool.evict_sterile().len(),
        0,
        "eviction is idempotent"
    );

    assert_eq!(harness.pool.replenish_blocking().expect("replenish"), 3);
    assert_eq!(harness.pool.occupancy().sterile, 3);
    let claim = harness
        .pool
        .claim(op(2), intent(2).fingerprint())
        .expect("claim");
    assert!(
        !evidence
            .iter()
            .any(|evicted| evicted.worker == claim.outcome.worker),
        "the rebuilt pool never returns an evicted worker"
    );
}
