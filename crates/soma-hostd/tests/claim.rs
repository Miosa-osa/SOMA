//! Single-winner, idempotent, conflicting, and never-reused claims.

#![cfg(unix)]

mod support;

use std::{
    sync::{Arc, Barrier},
    thread,
    time::Instant,
};

use soma_hostd::{
    ClaimClass, ClaimError, DestroyReason, ExhaustedBehavior, Limits, Phase, RecordKind,
    RequestFingerprint,
};
use support::{harness, intent, limits, op};

#[test]
fn exactly_one_of_100_concurrent_claimers_wins_one_sterile_worker_50_times() {
    let harness = harness(limits(1, 2));
    for round in 0..50_u32 {
        harness.pool.replenish_blocking().expect("replenish");
        assert_eq!(harness.pool.occupancy().sterile, 1);
        let barrier = Arc::new(Barrier::new(100));
        let handles: Vec<_> = (0..100_u32)
            .map(|index| {
                let pool = Arc::clone(&harness.pool);
                let barrier = Arc::clone(&barrier);
                let operation = op(round * 1000 + index);
                thread::spawn(move || {
                    barrier.wait();
                    let request = intent(round * 1000 + index);
                    let outcome = pool.claim(operation, request.fingerprint());
                    match outcome {
                        Ok(claim) => {
                            let granted = claim.grant.is_some();
                            if let Some(grant) = claim.grant {
                                pool.transfer(grant, &request).expect("transfer");
                            }
                            (granted, false)
                        }
                        Err(ClaimError::Exhausted(_)) => (false, true),
                        Err(other) => panic!("unexpected {other}"),
                    }
                })
            })
            .collect();
        let results: Vec<(bool, bool)> = handles
            .into_iter()
            .map(|handle| handle.join().expect("thread"))
            .collect();
        assert_eq!(
            results.iter().filter(|(won, _)| *won).count(),
            1,
            "round {round}"
        );
        assert_eq!(
            results.iter().filter(|(_, exhausted)| *exhausted).count(),
            99
        );
        let occupancy = harness.pool.occupancy();
        assert_eq!(occupancy.assigned, 1);
        assert_eq!(occupancy.sterile, 0);
        let assigned = harness
            .pool
            .ledger()
            .entries()
            .expect("entries")
            .into_values()
            .find(|entry| entry.phase == Phase::Assigned)
            .expect("assigned entry");
        harness.pool.release(assigned.worker).expect("release");
    }
}

#[test]
fn replay_with_the_same_fingerprint_returns_the_identical_outcome() {
    let harness = harness(limits(2, 2));
    harness.pool.replenish_blocking().expect("replenish");
    let fingerprint = intent(1).fingerprint();
    let first = harness.pool.claim(op(1), fingerprint).expect("claim");
    let outcome = first.outcome;
    assert_eq!(outcome.class, ClaimClass::Prepared);
    let grant = first.grant.expect("fresh winner holds the grant");
    let replay = harness.pool.claim(op(1), fingerprint).expect("replay");
    assert_eq!(replay.outcome, outcome);
    assert!(
        replay.grant.is_none(),
        "a replay never receives a second grant"
    );
    harness.pool.transfer(grant, &intent(1)).expect("transfer");
    assert_eq!(
        harness
            .pool
            .claim(op(1), fingerprint)
            .expect("replay")
            .outcome,
        outcome
    );
    harness.pool.release(outcome.worker).expect("release");
    assert_eq!(
        harness
            .pool
            .claim(op(1), fingerprint)
            .expect("replay")
            .outcome,
        outcome
    );
    assert_eq!(
        harness.pool.occupancy().sterile,
        1,
        "the replay took no second worker"
    );
    assert_eq!(harness.pool.ledger().claims().expect("claims").len(), 1);
}

#[test]
fn concurrent_replays_of_one_operation_all_receive_the_identical_outcome() {
    let harness = harness(limits(4, 4));
    harness.pool.replenish_blocking().expect("replenish");
    let barrier = Arc::new(Barrier::new(100));
    let handles: Vec<_> = (0..100)
        .map(|_| {
            let pool = Arc::clone(&harness.pool);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                let claim = pool.claim(op(7), intent(7).fingerprint()).expect("claim");
                let granted = claim.grant.is_some();
                if let Some(grant) = claim.grant {
                    pool.transfer(grant, &intent(7)).expect("transfer");
                }
                (claim.outcome, granted)
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|h| h.join().expect("thread"))
        .collect();
    assert_eq!(results.iter().filter(|(_, granted)| *granted).count(), 1);
    assert!(results.iter().all(|(outcome, _)| *outcome == results[0].0));
    assert_eq!(
        harness.pool.occupancy().sterile,
        3,
        "one operation took exactly one worker"
    );
}

#[test]
fn changed_intent_under_the_same_operation_conflicts() {
    let harness = harness(limits(2, 2));
    harness.pool.replenish_blocking().expect("replenish");
    let recorded = RequestFingerprint::of(b"first");
    let presented = RequestFingerprint::of(b"second");
    let claim = harness.pool.claim(op(3), recorded).expect("claim");
    assert_eq!(
        harness.pool.claim(op(3), presented).map(|c| c.outcome),
        Err(ClaimError::OperationConflict {
            operation: op(3),
            recorded,
            presented,
        })
    );
    drop(claim);
    assert_eq!(
        harness.pool.claim(op(3), presented).map(|c| c.outcome),
        Err(ClaimError::OperationConflict {
            operation: op(3),
            recorded,
            presented,
        }),
        "the binding outlives the worker"
    );
    let worker = harness
        .pool
        .ledger()
        .claims()
        .expect("claims")
        .first()
        .map(|claim| claim.worker)
        .expect("claim record");
    assert_eq!(
        harness.pool.inspect(worker).map(|v| v.phase),
        Some(Phase::Dead)
    );
    let records = harness.pool.ledger().records().expect("records");
    assert!(records.iter().any(|(_, record)| {
        record.kind == RecordKind::Destroying && record.detail == DestroyReason::Dropped as u8
    }));
}

#[test]
fn an_assigned_worker_is_never_claimable_again_even_after_release() {
    let harness = harness(limits(1, 1));
    harness.pool.replenish_blocking().expect("replenish");
    let claim = harness
        .pool
        .claim(op(1), intent(1).fingerprint())
        .expect("claim");
    let worker = claim.outcome.worker;
    harness
        .pool
        .transfer(claim.grant.expect("grant"), &intent(1))
        .expect("transfer");
    assert!(matches!(
        harness.pool.claim(op(2), intent(2).fingerprint()),
        Err(ClaimError::Exhausted(_))
    ));
    assert_eq!(
        harness.pool.inspect(worker).map(|v| v.phase),
        Some(Phase::Assigned)
    );
    harness.pool.start(worker).expect("start");
    harness.pool.release(worker).expect("release");
    assert_eq!(
        harness.pool.inspect(worker).map(|v| v.phase),
        Some(Phase::Dead)
    );
    assert!(matches!(
        harness.pool.claim(op(2), intent(2).fingerprint()),
        Err(ClaimError::Exhausted(_))
    ));
    harness.pool.replenish_blocking().expect("replenish");
    let second = harness
        .pool
        .claim(op(2), intent(2).fingerprint())
        .expect("claim");
    assert_ne!(
        second.outcome.worker, worker,
        "a fresh worker replaced the used one"
    );
    let entries = harness.pool.ledger().entries().expect("entries");
    assert_eq!(entries[&worker].phase, Phase::Dead);
    assert!(entries[&worker].was_assigned);
    assert!(
        harness
            .table
            .process(entries[&worker].identity.expect("identity").process)
            .is_some_and(|p| !p.alive)
    );
}

#[test]
fn an_exhausted_pool_rejects_immediately_without_queueing() {
    let harness = harness(Limits {
        target: 0,
        ..limits(0, 1)
    });
    let started = Instant::now();
    let error = harness
        .pool
        .claim(op(1), intent(1).fingerprint())
        .map(|c| c.outcome)
        .expect_err("exhausted");
    let ClaimError::Exhausted(exhausted) = error else {
        panic!("unexpected {error}");
    };
    assert!(started.elapsed() < harness.pool.limits().claim_deadline);
    assert_eq!(exhausted.occupancy.sterile, 0);
    assert_eq!(exhausted.max, 1);
    assert_eq!(exhausted.behavior, ExhaustedBehavior::Reject);
    assert!(
        harness.pool.claim(op(1), intent(1).fingerprint()).is_err(),
        "no binding was retained"
    );
    assert!(harness.pool.ledger().claims().expect("claims").is_empty());
}
