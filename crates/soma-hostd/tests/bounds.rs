//! Bounded replenishment, pool maximum, and 100-way fairness.

#![cfg(unix)]

mod support;

use std::{
    sync::{Arc, Barrier},
    thread,
    time::{Duration, Instant},
};

use soma_hostd::{
    ClaimError, ConstructionFault, Limits, OverloadGate, ReplenishLimit, testing::FaultPlan,
};
use support::{harness, intent, limits, op};

#[test]
fn a_replenishment_storm_never_exceeds_the_construction_concurrency() {
    let harness = harness(Limits {
        replenish_concurrency: 3,
        ..limits(40, 40)
    });
    harness.pool.launcher().set_plan(FaultPlan {
        construct_delay: Duration::from_millis(3),
        ..FaultPlan::default()
    });
    let barrier = Arc::new(Barrier::new(32));
    let handles: Vec<_> = (0..32)
        .map(|_| {
            let pool = Arc::clone(&harness.pool);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                let mut spawned = 0;
                for _ in 0..20 {
                    let report = pool.replenish();
                    spawned += report.spawned;
                    assert!(report.in_flight <= 3);
                    thread::sleep(Duration::from_millis(1));
                }
                spawned
            })
        })
        .collect();
    let spawned: usize = handles.into_iter().map(|h| h.join().expect("thread")).sum();
    loop {
        harness.pool.wait_replenishment();
        let report = harness.pool.replenish();
        if report.deficit == 0 && report.spawned == 0 {
            break;
        }
    }
    harness.pool.wait_replenishment();
    assert!(
        harness.pool.launcher().peak_concurrency() <= 3,
        "peak {}",
        harness.pool.launcher().peak_concurrency()
    );
    assert_eq!(harness.pool.occupancy().sterile, 40);
    assert_eq!(harness.pool.launcher().constructed(), 40);
    assert!(
        spawned <= 40,
        "{spawned} constructions were spawned for a deficit of 40"
    );
    let report = harness.pool.replenish();
    assert_eq!(report.spawned, 0);
    assert_eq!(report.limited_by, None);
}

#[test]
fn the_pool_maximum_and_concurrency_gates_are_typed() {
    let harness = harness(Limits {
        replenish_concurrency: 1,
        ..limits(2, 2)
    });
    harness.pool.replenish_blocking().expect("replenish");
    let failure = harness.pool.construct_one().expect_err("full");
    assert!(matches!(
        failure.fault,
        ConstructionFault::Overloaded(overloaded) if overloaded.gate == OverloadGate::PoolMaximum
    ));
    let report = harness.pool.replenish();
    assert_eq!(report.spawned, 0);
    for index in 0..2 {
        let claim = harness
            .pool
            .claim(op(index), intent(index).fingerprint())
            .expect("claim");
        harness
            .pool
            .transfer(claim.grant.expect("grant"), &intent(index))
            .expect("transfer");
    }
    let report = harness.pool.replenish();
    assert_eq!(report.limited_by, Some(ReplenishLimit::PoolMaximum));
    assert_eq!(report.spawned, 0);
    assert!(matches!(
        harness
            .pool
            .claim(op(9), intent(9).fingerprint())
            .map(|c| c.outcome),
        Err(ClaimError::Exhausted(_))
    ));
}

#[test]
fn one_hundred_operations_over_ten_workers_all_complete_without_starvation() {
    let harness = harness(Limits {
        replenish_concurrency: 10,
        ..limits(10, 10)
    });
    harness.pool.replenish_blocking().expect("replenish");
    let started = Instant::now();
    let barrier = Arc::new(Barrier::new(100));
    let handles: Vec<_> = (0..100_u32)
        .map(|index| {
            let pool = Arc::clone(&harness.pool);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                let mut retries = 0_u32;
                loop {
                    match pool.claim(op(index), intent(index).fingerprint()) {
                        Ok(claim) => {
                            let grant = claim.grant.expect("first claim of a fresh operation");
                            let evidence = pool.transfer(grant, &intent(index)).expect("transfer");
                            pool.start(evidence.worker).expect("start");
                            pool.release(evidence.worker).expect("release");
                            let _ = pool.replenish();
                            return (retries, evidence.worker);
                        }
                        Err(ClaimError::Exhausted(_)) => {
                            retries += 1;
                            assert!(retries < 20_000, "operation {index} starved");
                            let _ = pool.replenish();
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(other) => panic!("operation {index}: {other}"),
                    }
                }
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|h| h.join().expect("thread"))
        .collect();
    harness.pool.wait_replenishment();
    let elapsed = started.elapsed();
    let max_retries = results
        .iter()
        .map(|(retries, _)| *retries)
        .max()
        .unwrap_or(0);
    let mut workers: Vec<_> = results.iter().map(|(_, worker)| *worker).collect();
    workers.sort_unstable();
    workers.dedup();
    assert_eq!(
        workers.len(),
        100,
        "every operation got its own single-use worker"
    );
    let claims = harness.pool.ledger().claims().expect("claims");
    assert_eq!(claims.len(), 100);
    let mut operations: Vec<_> = claims.iter().map(|claim| claim.operation).collect();
    operations.sort_unstable();
    operations.dedup();
    assert_eq!(
        operations.len(),
        100,
        "each operation was claimed exactly once"
    );
    assert!(
        elapsed < Duration::from_secs(60),
        "100-way fairness took {elapsed:?}"
    );
    eprintln!("fairness: 100 operations over 10 workers in {elapsed:?}, max retries {max_retries}");
    assert_eq!(harness.table.alive(), harness.pool.occupancy().sterile);
}

#[test]
fn a_pass_that_ends_below_the_minimum_reports_the_urgency() {
    let harness = harness(Limits {
        min: 2,
        target: 3,
        replenish_concurrency: 1,
        ..limits(3, 4)
    });
    harness.pool.launcher().set_plan(FaultPlan {
        construct_delay: Duration::from_millis(20),
        ..FaultPlan::default()
    });
    let first = harness.pool.replenish();
    assert!(
        first.urgent,
        "an empty pool below its minimum reports the urgency"
    );
    assert_eq!(first.spawned, 1, "the concurrency bound still holds");
    loop {
        harness.pool.wait_replenishment();
        let report = harness.pool.replenish();
        if report.deficit == 0 && report.spawned == 0 {
            assert!(!report.urgent, "a replenished pool is no longer urgent");
            assert_eq!(harness.pool.occupancy().sterile, 3);
            break;
        }
    }
    for index in 0..2_u32 {
        let claim = harness
            .pool
            .claim(op(index), intent(index).fingerprint())
            .expect("claim");
        harness
            .pool
            .transfer(claim.grant.expect("grant"), &intent(index))
            .expect("transfer");
    }
    assert!(
        harness.pool.replenish().urgent,
        "one sterile worker is below the minimum of two"
    );
}
