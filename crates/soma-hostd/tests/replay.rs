//! Replay idempotency is durable: it survives a registry eviction and a restart.

#![cfg(unix)]

mod support;

use std::sync::Arc;

use soma_hostd::{ClaimError, Limits, RequestFingerprint};
use support::{Harness, harness, intent, limits, op, open};

fn claims_for(pool: &support::TestPool, operation: soma_hostd::OperationId) -> usize {
    pool.ledger()
        .claims()
        .expect("claims")
        .iter()
        .filter(|claim| claim.operation == operation)
        .count()
}

/// Runs one operation to a running Instance and returns the harness for a restart.
fn running_first_operation() -> (Harness, soma_hostd::WorkerId) {
    let harness = harness(limits(2, 4));
    harness.pool.replenish_blocking().expect("replenish");
    let worker = {
        let claim = harness
            .pool
            .claim(op(1), intent(1).fingerprint())
            .expect("claim");
        let worker = claim.outcome.worker;
        harness
            .pool
            .transfer(claim.grant.expect("grant"), &intent(1))
            .expect("transfer");
        worker
    };
    harness.pool.start(worker).expect("start");
    (harness, worker)
}

#[test]
fn a_replay_after_a_restart_returns_the_recorded_outcome_and_takes_no_second_worker() {
    let (first, worker) = running_first_operation();
    let outcome = first
        .pool
        .claim(op(1), intent(1).fingerprint())
        .expect("replay")
        .outcome;
    let table = Arc::clone(&first.table);
    let dir = first.dir;
    drop(first.pool);

    let second = open(dir.path(), &table, limits(2, 4));
    second.reconcile().expect("reconcile");
    second.replenish_blocking().expect("replenish");
    let sterile = second.occupancy().sterile;
    let replay = second
        .claim(op(1), intent(1).fingerprint())
        .expect("replay");
    assert!(
        replay.grant.is_none(),
        "a replay after a restart never receives a second grant"
    );
    assert_eq!(replay.outcome, outcome, "the recorded outcome is identical");
    assert_eq!(replay.outcome.worker, worker);
    assert_eq!(
        second.occupancy().sterile,
        sterile,
        "the replay took no sterile worker"
    );
    assert_eq!(
        claims_for(&second, op(1)),
        1,
        "one Claiming record per operation"
    );
}

#[test]
fn a_changed_intent_after_a_restart_still_conflicts() {
    let (first, _) = running_first_operation();
    let recorded = intent(1).fingerprint();
    let table = Arc::clone(&first.table);
    let dir = first.dir;
    drop(first.pool);

    let second = open(dir.path(), &table, limits(2, 4));
    second.reconcile().expect("reconcile");
    second.replenish_blocking().expect("replenish");
    let presented = RequestFingerprint::of(b"changed after the restart");
    assert_eq!(
        second.claim(op(1), presented).map(|claim| claim.outcome),
        Err(ClaimError::OperationConflict {
            operation: op(1),
            recorded,
            presented,
        })
    );
    assert_eq!(claims_for(&second, op(1)), 1);
    assert_eq!(
        second
            .claim(op(1), recorded)
            .expect("replay")
            .outcome
            .operation,
        op(1),
        "the original intent still replays"
    );
}

#[test]
fn an_evicted_binding_is_recovered_from_the_ledger_instead_of_granting_a_second_worker() {
    let harness = harness(Limits {
        binding_limit: 2,
        ..limits(2, 2)
    });
    let mut outcomes = Vec::new();
    for index in 1..=2_u32 {
        harness.pool.replenish_blocking().expect("replenish");
        let claim = harness
            .pool
            .claim(op(index), intent(index).fingerprint())
            .expect("claim");
        outcomes.push(claim.outcome);
        harness
            .pool
            .transfer(claim.grant.expect("grant"), &intent(index))
            .expect("transfer");
        harness.pool.release(claim.outcome.worker).expect("release");
    }
    harness.pool.replenish_blocking().expect("replenish");
    let third = harness
        .pool
        .claim(op(3), intent(3).fingerprint())
        .expect("the registry evicts a completed binding to make room");
    drop(third);

    let sterile = harness.pool.occupancy().sterile;
    let replay = harness
        .pool
        .claim(op(1), intent(1).fingerprint())
        .expect("replay");
    assert!(
        replay.grant.is_none(),
        "an evicted binding never grants a second worker"
    );
    assert_eq!(replay.outcome, outcomes[0]);
    assert_eq!(harness.pool.occupancy().sterile, sterile);
    assert_eq!(claims_for(&harness.pool, op(1)), 1);
    let presented = RequestFingerprint::of(b"changed after the eviction");
    assert!(matches!(
        harness.pool.claim(op(1), presented),
        Err(ClaimError::OperationConflict { .. })
    ));
}
