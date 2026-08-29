//! Every claim passes the host capacity admission before a worker is granted.

#![cfg(unix)]

mod support;

use std::sync::Arc;

use soma_hostd::{
    Admission, ClaimError, Gate, ReconcileDisposition, SingleNode, testing::ProcessTable,
};
use support::{admission, host_profile, intent, ledger_dir, limits, op, open_with};

#[test]
fn a_claim_no_capacity_admits_is_refused_by_gate_and_grants_no_worker() {
    let mut profile = host_profile();
    profile.limits.resident_instances = 1;
    let admission = Arc::new(Admission::new(
        profile.validate().expect("profile"),
        SingleNode,
    ));
    let dir = ledger_dir();
    let table = ProcessTable::new();
    let pool = open_with(dir.path(), &table, limits(2, 2), &admission);
    pool.replenish_blocking().expect("replenish");
    let first = pool.claim(op(1), intent(1).fingerprint()).expect("claim");
    let worker = first.outcome.worker;
    pool.transfer(first.grant.expect("grant"), &intent(1))
        .expect("transfer");
    assert_eq!(admission.usage().residents, 1);

    let refused = pool.claim(op(2), intent(2).fingerprint());
    let Err(ClaimError::Capacity(rejection)) = refused.map(|claim| claim.outcome) else {
        panic!("the second claim must be refused by a capacity gate");
    };
    assert_eq!(rejection.gate, Gate::OperatorSafetyLimit);
    assert_eq!(rejection.limit, 1);
    assert_eq!(
        pool.occupancy().sterile,
        1,
        "a refused claim grants no worker"
    );
    assert!(
        pool.ledger()
            .claims()
            .expect("claims")
            .iter()
            .all(|claim| claim.operation != op(2)),
        "a refused claim records nothing"
    );

    pool.release(worker).expect("release");
    assert_eq!(
        admission.usage().residents,
        0,
        "release returns every dimension"
    );
    assert_eq!(admission.usage().launches, 0);
    let second = pool.claim(op(2), intent(2).fingerprint()).expect("claim");
    assert!(second.grant.is_some());
}

#[test]
fn a_transferred_claim_holds_no_launch_slot_and_a_failed_one_returns_its_reservation() {
    let harness = support::harness(limits(2, 6));
    harness.pool.replenish_blocking().expect("replenish");
    let claim = harness
        .pool
        .claim(op(1), intent(1).fingerprint())
        .expect("claim");
    assert_eq!(harness.admission.usage().launches, 1, "the Launch is open");
    harness
        .pool
        .transfer(claim.grant.expect("grant"), &intent(1))
        .expect("transfer");
    assert_eq!(
        harness.admission.usage().launches,
        0,
        "the Launch slot is returned at commit"
    );
    assert_eq!(harness.admission.usage().residents, 1);

    let dropped = harness
        .pool
        .claim(op(2), intent(2).fingerprint())
        .expect("claim");
    drop(dropped);
    assert_eq!(
        harness.admission.usage().residents,
        1,
        "a dropped grant returns its reservation"
    );
    harness.pool.replenish_blocking().expect("replenish");
    let mismatched = harness
        .pool
        .claim(op(3), intent(3).fingerprint())
        .expect("claim");
    harness
        .pool
        .transfer(mismatched.grant.expect("grant"), &intent(4))
        .expect_err("intent mismatch");
    assert_eq!(
        harness.admission.usage().residents,
        1,
        "a failed transfer returns its reservation"
    );
}

#[test]
fn a_restart_rebuilds_the_committed_capacity_of_every_retained_instance() {
    let first = admission();
    let dir = ledger_dir();
    let table = ProcessTable::new();
    {
        let pool = open_with(dir.path(), &table, limits(1, 2), &first);
        pool.replenish_blocking().expect("replenish");
        let worker = {
            let claim = pool.claim(op(1), intent(1).fingerprint()).expect("claim");
            let worker = claim.outcome.worker;
            pool.transfer(claim.grant.expect("grant"), &intent(1))
                .expect("transfer");
            worker
        };
        pool.start(worker).expect("start");
    }
    assert_eq!(first.usage().residents, 1);

    let restarted = admission();
    let second = open_with(dir.path(), &table, limits(1, 2), &restarted);
    assert_eq!(
        restarted.usage().residents,
        0,
        "a fresh process commits nothing before it reconciles"
    );
    let report = second.reconcile().expect("reconcile");
    assert_eq!(
        restarted.usage().residents,
        1,
        "reconciliation rebuilds the usage of the retained Instance"
    );
    assert_eq!(
        restarted.usage().launches,
        0,
        "a retained Instance holds no Launch slot"
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.disposition == ReconcileDisposition::Retained && finding.capacity_restored
        }),
        "the retained finding reports its restored capacity"
    );
    let running = report
        .findings
        .iter()
        .find(|finding| finding.disposition == ReconcileDisposition::Retained)
        .expect("retained")
        .worker;
    second.release(running).expect("release");
    assert_eq!(restarted.usage().residents, 0);
}
