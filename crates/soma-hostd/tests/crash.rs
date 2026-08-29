//! A crash between the broker's assignment and the commit still releases the leased head
//! and the network bundle at the next reconciliation.

#![cfg(unix)]

mod support;

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use soma_hostd::{
    Phase, TransferStep,
    testing::{FaultPlan, InProcessBroker, InjectedFault, ProcessTable},
};
use support::{admission, intent, ledger_dir, limits, op, open_shared};

/// Crashes one transfer at `step` and reconciles a fresh pool over the same host state.
fn crash_at(step: TransferStep, operation: u32) {
    let broker = Arc::new(InProcessBroker::new());
    let admission = admission();
    let table = ProcessTable::new();
    let dir = ledger_dir();
    let worker = {
        let pool = open_shared(dir.path(), &table, limits(1, 2), &admission, &broker);
        pool.launcher().set_plan(FaultPlan {
            transfer: Some((step, InjectedFault::Abandon)),
            ..FaultPlan::default()
        });
        pool.replenish_blocking().expect("replenish");
        let claim = pool
            .claim(op(operation), intent(operation).fingerprint())
            .expect("claim");
        let worker = claim.outcome.worker;
        let grant = claim.grant.expect("grant");
        let crashed = catch_unwind(AssertUnwindSafe(|| {
            let _ = pool.transfer(grant, &intent(operation));
        }));
        assert!(crashed.is_err(), "the transfer must die, not fail cleanly");
        worker
    };
    assert_eq!(
        broker.leased_heads(),
        1,
        "{step:?}: the crash left the Instance head leased"
    );

    let restarted = open_shared(dir.path(), &table, limits(1, 2), &admission, &broker);
    let entry = restarted.ledger().entries().expect("entries")[&worker].clone();
    assert_eq!(entry.phase, Phase::Claiming);
    let report = restarted.reconcile().expect("reconcile");
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.worker == worker)
        .expect("the crashed worker is a suspect");
    assert!(finding.complete, "{step:?}: reconciliation completed");
    assert_eq!(
        broker.leased_heads(),
        0,
        "{step:?}: reconciliation released the head leased under the Instance token"
    );
    assert_eq!(
        broker.live_bundles(),
        0,
        "{step:?}: reconciliation released the network bundle"
    );
    assert_eq!(
        table.alive(),
        0,
        "{step:?}: the worker process was terminated"
    );
}

#[test]
fn a_crash_before_the_first_frame_releases_the_assigned_resources() {
    crash_at(TransferStep::Identity, 1);
}

#[test]
fn a_crash_between_frames_releases_the_assigned_resources() {
    crash_at(TransferStep::Network, 2);
}
