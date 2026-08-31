//! The daemon answers each request from the real disposition of the worker behind it.

#![cfg(target_os = "linux")]

mod support;

use soma_hostd::{
    FailureCode, Reply, TransferStep, daemon, failure_code,
    testing::{FaultPlan, InjectedFault},
};
use support::{claim_request, harness, intent, limits, op};

#[test]
fn a_replay_of_an_operation_whose_transfer_failed_never_names_a_live_worker() {
    let harness = harness(limits(2, 6));
    harness.pool.launcher().set_plan(FaultPlan {
        transfer: Some((TransferStep::Disk, InjectedFault::Closed)),
        ..FaultPlan::default()
    });
    harness.pool.replenish_blocking().expect("replenish");
    let failed = daemon::handle(&harness.runtime, claim_request(1));
    assert_eq!(
        failed,
        Reply::Failed(failure_code(FailureCode::Transfer)),
        "the first attempt reports the transfer failure"
    );
    let worker = harness
        .pool
        .ledger()
        .claims()
        .expect("claims")
        .first()
        .expect("one claim")
        .worker;
    assert!(
        harness
            .pool
            .inspect(worker)
            .is_none_or(|view| view.phase.is_terminal()),
        "the worker of the failed transfer was destroyed"
    );

    harness.pool.launcher().set_plan(FaultPlan::default());
    harness.pool.wait_replenishment();
    harness.pool.replenish_blocking().expect("replenish");
    assert_eq!(
        daemon::handle(&harness.runtime, claim_request(1)),
        Reply::Failed(failure_code(FailureCode::Terminated)),
        "the retry learns the Launch failed instead of being told it holds a dead worker"
    );
}

#[test]
fn a_replay_of_a_live_operation_repeats_its_launch_page_and_a_released_one_is_terminal() {
    let harness = harness(limits(2, 6));
    harness.pool.replenish_blocking().expect("replenish");
    let first = daemon::handle(&harness.runtime, claim_request(2));
    let Reply::Claimed { worker, .. } = first else {
        panic!("the first claim transfers authority");
    };
    assert_eq!(
        daemon::handle(&harness.runtime, claim_request(2)),
        first,
        "a retry after a lost reply receives the identical launch page"
    );
    harness.pool.release(worker).expect("release");
    assert_eq!(
        daemon::handle(&harness.runtime, claim_request(2)),
        Reply::Failed(failure_code(FailureCode::Terminated))
    );
    assert_eq!(
        harness
            .pool
            .ledger()
            .claims()
            .expect("claims")
            .iter()
            .filter(|claim| claim.operation == op(2))
            .count(),
        1,
        "every replay stayed on the one recorded claim"
    );
    let _ = intent(2);
}
