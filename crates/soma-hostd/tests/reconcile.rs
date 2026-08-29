//! Restart reconciliation over a durable ledger and a surviving process table.

#![cfg(unix)]

mod support;

use std::sync::{Arc, Barrier};

use soma_hostd::{
    ConstructionFault, Liveness, Phase, ReconcileDisposition, ReplenishLimit, ResourceLiveness,
};
use support::{harness, intent, limits, op, open};

#[test]
fn a_restart_marks_every_nonterminal_entry_suspect_and_reconciles_before_replenishing() {
    let first = harness(limits(4, 8));
    let (assigned, running, claiming_worker) = {
        let pool = &first.pool;
        pool.replenish_blocking().expect("replenish");
        let assigned = pool.claim(op(1), intent(1).fingerprint()).expect("claim");
        let assigned = pool
            .transfer(assigned.grant.expect("grant"), &intent(1))
            .expect("transfer")
            .worker;
        let running = pool.claim(op(2), intent(2).fingerprint()).expect("claim");
        let running = pool
            .transfer(running.grant.expect("grant"), &intent(2))
            .expect("transfer")
            .worker;
        pool.start(running).expect("start");
        let claiming = pool.claim(op(3), intent(3).fingerprint()).expect("claim");
        let claiming_worker = claiming.outcome.worker;
        pool.replenish_blocking().expect("replenish");
        assert_eq!(pool.occupancy().sterile, 4);
        std::mem::forget(claiming.grant);
        (assigned, running, claiming_worker)
    };
    let table = Arc::clone(&first.table);
    let dir = first.dir;
    drop(first.pool);
    assert_eq!(table.alive(), 7, "processes outlive the allocator");

    let second = open(dir.path(), &table, limits(4, 8));
    assert!(second.needs_reconcile());
    assert_eq!(
        second.replenish().limited_by,
        Some(ReplenishLimit::Unreconciled)
    );
    assert!(matches!(
        second.construct_one().expect_err("unreconciled").fault,
        ConstructionFault::Unreconciled
    ));
    let report = second.reconcile().expect("reconcile");
    assert_eq!(report.suspects, 7);
    let (terminated, released, retained) = report.counts();
    assert_eq!((terminated, released, retained), (6, 0, 1));
    let finding = |worker| {
        report
            .findings
            .iter()
            .find(|f| f.worker == worker)
            .expect("finding")
    };
    assert_eq!(finding(assigned).phase, Phase::Assigned);
    assert_eq!(
        finding(assigned).disposition,
        ReconcileDisposition::Terminated
    );
    assert_eq!(finding(assigned).liveness, Some(Liveness::Alive));
    assert_eq!(finding(claiming_worker).phase, Phase::Claiming);
    assert_eq!(
        finding(claiming_worker).disposition,
        ReconcileDisposition::Terminated
    );
    assert_eq!(finding(running).disposition, ReconcileDisposition::Retained);
    assert_eq!(
        finding(running).resources,
        ResourceLiveness::Absent,
        "a new broker knows no old lease"
    );
    assert!(report.findings.iter().all(|f| f.complete));
    assert_eq!(table.alive(), 1, "only the running Instance survived");
    assert!(!second.needs_reconcile());
    assert_eq!(
        second.inspect(running).map(|v| v.phase),
        Some(Phase::Running)
    );
    assert_eq!(second.replenish_blocking().expect("replenish"), 4);
    assert_eq!(second.occupancy().sterile, 4);
    let released = second.release(running).expect("release");
    assert!(released.destroyed.complete);
    assert_eq!(table.alive(), 4);
    let entries = second.ledger().entries().expect("entries");
    assert_eq!(entries[&running].phase, Phase::Dead);
    assert_eq!(entries[&assigned].phase, Phase::Dead);
    assert!(entries[&assigned].suspect);
    assert_eq!(
        entries
            .values()
            .filter(|e| e.phase == Phase::Sterile)
            .count(),
        4
    );
    drop(second);

    let third = open(dir.path(), &table, limits(4, 8));
    assert!(
        third.needs_reconcile(),
        "the four sterile workers of the second pool are suspect"
    );
    let report = third.reconcile().expect("reconcile");
    assert_eq!(report.suspects, 4);
    assert_eq!(table.alive(), 0);
}

#[test]
fn gone_processes_are_released_rather_than_terminated() {
    let first = harness(limits(2, 2));
    {
        let pool = &first.pool;
        pool.replenish_blocking().expect("replenish");
        let claim = pool.claim(op(1), intent(1).fingerprint()).expect("claim");
        pool.transfer(claim.grant.expect("grant"), &intent(1))
            .expect("transfer");
    }
    let table = Arc::clone(&first.table);
    let dir = first.dir;
    drop(first.pool);
    table.kill_all();
    let second = open(dir.path(), &table, limits(2, 2));
    let report = second.reconcile().expect("reconcile");
    assert_eq!(report.suspects, 2);
    assert_eq!(report.counts(), (0, 2, 0));
    assert!(
        report
            .findings
            .iter()
            .all(|f| f.liveness == Some(Liveness::Gone) && f.complete)
    );
    let clean = open(dir.path(), &table, limits(2, 2));
    assert!(!clean.needs_reconcile());
    assert_eq!(clean.reconcile().expect("reconcile").suspects, 0);
}

#[test]
fn concurrent_reconciliations_adopt_one_running_instance_exactly_once() {
    let first = harness(limits(1, 4));
    let running = {
        let pool = &first.pool;
        pool.replenish_blocking().expect("replenish");
        let worker = {
            let claim = pool.claim(op(1), intent(1).fingerprint()).expect("claim");
            let worker = claim.outcome.worker;
            pool.transfer(claim.grant.expect("grant"), &intent(1))
                .expect("transfer");
            worker
        };
        pool.start(worker).expect("start");
        worker
    };
    let table = Arc::clone(&first.table);
    let dir = first.dir;
    drop(first.pool);

    let second = open(dir.path(), &table, limits(1, 4));
    let barrier = Arc::new(Barrier::new(2));
    let passes: Vec<_> = (0..2)
        .map(|_| {
            let pool = Arc::clone(&second);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                pool.reconcile().expect("reconcile")
            })
        })
        .collect();
    let reports: Vec<_> = passes
        .into_iter()
        .map(|pass| pass.join().expect("thread"))
        .collect();
    let retained: usize = reports.iter().map(|report| report.counts().2).sum();
    assert_eq!(retained, 1, "the Instance was adopted exactly once");
    assert_eq!(
        second.occupancy().running,
        1,
        "one running Instance produced one slot"
    );
    second.release(running).expect("release");
    let occupancy = second.occupancy();
    assert_eq!(occupancy.running, 0, "no zombie slot outlived the release");
    assert!(second.release(running).is_err());
    assert_eq!(second.replenish_blocking().expect("replenish"), 1);
}
