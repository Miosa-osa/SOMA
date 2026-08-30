//! Exactly-once transfer: every step fault, every ambiguity, destroys the worker.

#![cfg(unix)]

mod support;

use std::time::Duration;

use soma_hostd::{
    DestroyReason, Limits, Phase, RecordKind, Resource, ResourceFault, ResourceFaultKind,
    StartFault, TransferFault, TransferStep,
    testing::{FaultPlan, InjectedFault},
};
use support::{harness, intent, limits, op};

#[test]
fn a_successful_transfer_delivers_eight_frames_then_starts_and_releases() {
    let harness = harness(limits(1, 1));
    harness.pool.replenish_blocking().expect("replenish");
    let claim = harness
        .pool
        .claim(op(1), intent(1).fingerprint())
        .expect("claim");
    let evidence = harness
        .pool
        .transfer(claim.grant.expect("grant"), &intent(1))
        .expect("transfer");
    assert_eq!(evidence.steps, 8);
    assert_eq!(evidence.launch.vsock_cid(), intent(1).vsock_cid);
    let entry = harness.pool.ledger().entries().expect("entries")[&evidence.worker].clone();
    assert_eq!(entry.phase, Phase::Assigned);
    assert_eq!(entry.last_step, Some(TransferStep::Commit));
    assert_eq!(entry.instance, Some(intent(1).instance));
    let process = harness
        .table
        .process(entry.identity.expect("identity").process)
        .expect("process");
    assert_eq!(process.received, TransferStep::ALL.to_vec());
    assert_eq!(process.descriptors, 3);
    assert_eq!(harness.pool.broker().leased_heads(), 1);
    harness.pool.start(evidence.worker).expect("start");
    assert_eq!(
        harness.pool.inspect(evidence.worker).map(|v| v.phase),
        Some(Phase::Running)
    );
    let released = harness.pool.release(evidence.worker).expect("release");
    assert_eq!(released.reason, DestroyReason::Released);
    assert!(released.destroyed.complete && released.released.complete && released.ledger);
    assert_eq!(harness.pool.broker().leased_heads(), 0);
    assert_eq!(harness.pool.broker().live_bundles(), 0);
    assert_eq!(harness.table.alive(), 0);
}

#[test]
fn a_fault_at_every_transfer_step_destroys_the_worker_with_a_ledger_disposition() {
    let faults = [
        (InjectedFault::Rejected, TransferFault::Rejected),
        (InjectedFault::Timeout, TransferFault::Timeout),
        (InjectedFault::PartialAck, TransferFault::PartialAck),
        (InjectedFault::Closed, TransferFault::Closed),
    ];
    let harness = harness(limits(1, 1));
    let mut round = 0;
    for step in TransferStep::ALL {
        for (injected, expected) in faults {
            round += 1;
            harness.pool.launcher().set_plan(FaultPlan {
                transfer: Some((step, injected)),
                ..FaultPlan::default()
            });
            harness.pool.replenish_blocking().expect("replenish");
            let claim = harness
                .pool
                .claim(op(round), intent(round).fingerprint())
                .expect("claim");
            let worker = claim.outcome.worker;
            let before = harness.pool.broker().counters();
            let failure = harness
                .pool
                .transfer(claim.grant.expect("grant"), &intent(round))
                .expect_err("injected fault");
            assert_eq!(failure.worker, worker);
            assert_eq!(failure.step, Some(step), "{step:?} {injected:?}");
            assert_eq!(failure.fault, expected);
            assert!(failure.disposition.destroyed.complete);
            assert!(failure.disposition.released.complete);
            assert_eq!(
                harness.pool.inspect(worker).map(|v| v.phase),
                Some(Phase::Dead)
            );
            let after = harness.pool.broker().counters();
            assert_eq!(
                after.released,
                before.released + 1,
                "assigned refs were released"
            );
            assert_eq!(harness.pool.broker().leased_heads(), 0);
            assert_eq!(harness.pool.broker().live_bundles(), 0);
            assert_eq!(harness.table.alive(), 0);
            let entry = harness.pool.ledger().entries().expect("entries")[&worker].clone();
            assert_eq!(entry.phase, Phase::Dead);
            assert!(!entry.was_assigned);
            let previous = TransferStep::from_code(step.code() - 1);
            assert_eq!(
                entry.last_step, previous,
                "acknowledged steps stop before {step:?}"
            );
            let records: Vec<_> = harness
                .pool
                .ledger()
                .records()
                .expect("records")
                .into_iter()
                .filter(|(_, record)| record.worker == worker)
                .map(|(_, record)| (record.kind, record.detail))
                .collect();
            assert!(records.contains(&(RecordKind::TransferFault, step.code())));
            assert!(
                records.contains(&(RecordKind::Destroying, DestroyReason::TransferFault as u8))
            );
            assert_eq!(
                records.last().map(|(kind, _)| *kind),
                Some(RecordKind::Dead)
            );
            assert!(
                !records
                    .iter()
                    .any(|(kind, _)| *kind == RecordKind::Assigned)
            );
        }
    }
    assert_eq!(harness.pool.occupancy().sterile, 0);
}

#[test]
fn a_stall_past_the_claim_deadline_destroys_the_worker() {
    let harness = harness(Limits {
        claim_deadline: Duration::from_millis(500),
        ..limits(1, 1)
    });
    harness.pool.launcher().set_plan(FaultPlan {
        transfer: Some((
            TransferStep::Disk,
            InjectedFault::Stall(Duration::from_secs(1)),
        )),
        ..FaultPlan::default()
    });
    harness.pool.replenish_blocking().expect("replenish");
    let claim = harness
        .pool
        .claim(op(1), intent(1).fingerprint())
        .expect("claim");
    let worker = claim.outcome.worker;
    let failure = harness
        .pool
        .transfer(claim.grant.expect("grant"), &intent(1))
        .expect_err("deadline");
    assert_eq!(failure.step, Some(TransferStep::Network));
    assert_eq!(failure.fault, TransferFault::ClaimDeadline);
    assert_eq!(
        harness.pool.inspect(worker).map(|v| v.phase),
        Some(Phase::Dead)
    );
    let records = harness.pool.ledger().records().expect("records");
    assert!(records.iter().any(|(_, record)| {
        record.kind == RecordKind::Destroying && record.detail == DestroyReason::ClaimDeadline as u8
    }));
}

#[test]
fn a_stall_in_the_last_frame_destroys_the_worker_instead_of_assigning_it() {
    let harness = harness(Limits {
        claim_deadline: Duration::from_millis(500),
        ..limits(1, 1)
    });
    harness.pool.launcher().set_plan(FaultPlan {
        transfer: Some((
            TransferStep::Commit,
            InjectedFault::Stall(Duration::from_secs(1)),
        )),
        ..FaultPlan::default()
    });
    harness.pool.replenish_blocking().expect("replenish");
    let claim = harness
        .pool
        .claim(op(1), intent(1).fingerprint())
        .expect("claim");
    let worker = claim.outcome.worker;
    let failure = harness
        .pool
        .transfer(claim.grant.expect("grant"), &intent(1))
        .expect_err("the claim deadline passed during the last frame");
    assert_eq!(failure.step, Some(TransferStep::Commit));
    assert_eq!(failure.fault, TransferFault::ClaimDeadline);
    assert_eq!(
        harness.pool.inspect(worker).map(|view| view.phase),
        Some(Phase::Dead)
    );
    let records = harness.pool.ledger().records().expect("records");
    assert!(records.iter().any(|(_, record)| {
        record.kind == RecordKind::Destroying && record.detail == DestroyReason::ClaimDeadline as u8
    }));
    assert!(
        !records
            .iter()
            .any(|(_, record)| record.kind == RecordKind::Assigned),
        "an overrun transfer never assigns the worker"
    );
    assert_eq!(harness.pool.broker().leased_heads(), 0);
}

#[test]
fn a_resource_assignment_fault_destroys_the_worker_before_any_frame() {
    let harness = harness(limits(1, 1));
    harness.pool.replenish_blocking().expect("replenish");
    let fault = ResourceFault {
        resource: Resource::Network,
        kind: ResourceFaultKind::Exhausted,
    };
    harness.pool.broker().fail_assign(Some(fault));
    let claim = harness
        .pool
        .claim(op(1), intent(1).fingerprint())
        .expect("claim");
    let worker = claim.outcome.worker;
    let failure = harness
        .pool
        .transfer(claim.grant.expect("grant"), &intent(1))
        .expect_err("resource fault");
    assert_eq!(failure.step, None);
    assert_eq!(failure.fault, TransferFault::Resource(fault));
    assert_eq!(
        harness.pool.inspect(worker).map(|v| v.phase),
        Some(Phase::Dead)
    );
    assert_eq!(harness.pool.broker().live_bundles(), 0);
    assert!(harness.table.process(1000).is_some_and(|p| !p.alive));
}

#[test]
fn a_transfer_with_a_different_intent_than_the_claim_is_refused_and_destroys() {
    let harness = harness(limits(1, 1));
    harness.pool.replenish_blocking().expect("replenish");
    let claim = harness
        .pool
        .claim(op(1), intent(1).fingerprint())
        .expect("claim");
    let worker = claim.outcome.worker;
    let failure = harness
        .pool
        .transfer(claim.grant.expect("grant"), &intent(2))
        .expect_err("mismatch");
    assert_eq!(failure.fault, TransferFault::Rejected);
    assert_eq!(failure.step, None);
    assert_eq!(
        harness.pool.inspect(worker).map(|v| v.phase),
        Some(Phase::Dead)
    );
    let records = harness.pool.ledger().records().expect("records");
    assert!(records.iter().any(|(_, record)| {
        record.kind == RecordKind::Destroying
            && record.detail == DestroyReason::IntentMismatch as u8
    }));
}

#[test]
fn a_start_fault_destroys_the_assigned_worker() {
    let harness = harness(limits(1, 1));
    harness.pool.launcher().set_plan(FaultPlan {
        start: Some(StartFault::Rejected),
        ..FaultPlan::default()
    });
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
        harness.pool.start(worker),
        Err(soma_hostd::LifecycleError::Start(StartFault::Rejected))
    ));
    assert_eq!(
        harness.pool.inspect(worker).map(|v| v.phase),
        Some(Phase::Dead)
    );
    assert!(
        harness.pool.release(worker).is_err(),
        "nothing is owned after the start fault"
    );
}
