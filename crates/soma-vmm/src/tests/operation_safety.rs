use crate::{
    Argument, CleanupEvidence, Execute, FailureKind, FailurePhase, InstanceId, Machine,
    OperationId, Program, Recovery, Stop, operation::MAX_OPERATION_RECEIPTS,
};

use super::{
    fixtures::{execute, execution_limits, launch, stop},
    platform::{DeterministicPlatform, TestStage},
};

#[test]
fn rejected_execute_operations_cannot_exhaust_launch_capacity() {
    let mut machine = Machine::with_platform(DeterministicPlatform::healthy());

    for sequence in 0..MAX_OPERATION_RECEIPTS {
        let operation_id = (sequence as u128 + 100).to_le_bytes();
        let failure = machine
            .execute(execute(operation_id))
            .expect_err("Execute before Launch is rejected before admission");
        assert_eq!(failure.kind(), FailureKind::InvalidLifecycle);
    }

    machine
        .launch(launch())
        .expect("rejected operations retain no receipt capacity");
}

#[test]
fn rejected_stop_operations_cannot_consume_the_cleanup_slot() {
    let mut machine = Machine::with_platform(DeterministicPlatform::healthy());
    machine.launch(launch()).expect("launch is ready");

    for sequence in 0..MAX_OPERATION_RECEIPTS {
        let request = Stop::new(
            OperationId::new((sequence as u128 + 10_000).to_le_bytes()).expect("operation ID"),
            InstanceId::new([0x77; 16]).expect("wrong Instance ID"),
        );
        let failure = machine
            .stop(request)
            .expect_err("wrong Instance must be rejected before Stop admission");
        assert_eq!(failure.kind(), FailureKind::InstanceMismatch);
    }

    machine
        .stop(stop([0xff; 16]))
        .expect("the matching Instance retains its canonical Stop slot");
}

#[test]
fn incomplete_stop_replay_continues_only_the_same_cleanup_operation() {
    let mut machine = Machine::with_platform(DeterministicPlatform::failing_stop_once());
    machine.launch(launch()).expect("launch is ready");
    let accepted = stop([0x61; 16]);

    let first = machine
        .stop(accepted.clone())
        .expect_err("first cleanup attempt is incomplete");
    assert_eq!(first.kind(), FailureKind::StopFailed);
    assert_eq!(first.cleanup(), CleanupEvidence::Incomplete);

    let conflicting = machine
        .stop(stop([0x62; 16]))
        .expect_err("another Stop cannot take over accepted cleanup");
    assert_eq!(conflicting.kind(), FailureKind::OperationConflict);

    let stopped = machine
        .stop(accepted)
        .expect("exact replay continues idempotent cleanup");
    assert_eq!(stopped.cleanup(), CleanupEvidence::Complete);
}

#[test]
fn fatal_execute_failure_invalidates_the_guest_session() {
    let mut machine =
        Machine::with_platform(DeterministicPlatform::failing_at(TestStage::UserExecute));
    machine.launch(launch()).expect("launch is ready");

    let fatal = machine
        .execute(execute([0x71; 16]))
        .expect_err("transport failure is fatal to the guest session");
    assert_eq!(fatal.kind(), FailureKind::ExecuteFailed);
    assert_eq!(fatal.recovery(), Recovery::ReplaceMachine);

    let after = machine
        .execute(execute([0x72; 16]))
        .expect_err("a fatal channel failure leaves the Machine failed");
    assert_eq!(after.kind(), FailureKind::InvalidLifecycle);
    assert_eq!(after.phase(), FailurePhase::Lifecycle);
}

#[test]
fn command_debug_output_redacts_guest_controlled_bytes() {
    let request = execute([0x73; 16]);
    let debug = format!("{request:?}");

    assert!(!debug.contains("/usr/bin/true"));
    assert!(!debug.contains("--version"));
    assert!(debug.contains("program_bytes"));
    assert!(debug.contains("argument_bytes"));
}

#[test]
fn reusing_an_operation_id_for_different_request_bytes_conflicts() {
    let mut machine = Machine::with_platform(DeterministicPlatform::healthy());
    machine.launch(launch()).expect("launch is ready");

    let failure = machine
        .execute(execute([1; 16]))
        .expect_err("the Launch operation ID cannot be reused for Execute");

    assert_eq!(failure.kind(), FailureKind::OperationConflict);
    assert_eq!(failure.phase(), FailurePhase::Idempotency);
    assert_eq!(failure.recovery(), Recovery::DoNotRetry);
    assert_eq!(failure.cleanup(), CleanupEvidence::NotRequired);
}

#[test]
fn reusing_an_execute_operation_id_with_different_arguments_conflicts() {
    let mut machine = Machine::with_platform(DeterministicPlatform::healthy());
    machine.launch(launch()).expect("launch is ready");
    let operation_id = OperationId::new([10; 16]).expect("operation ID");
    let first = Execute::new(
        operation_id,
        InstanceId::new([2; 16]).expect("instance ID"),
        Program::new(b"/usr/bin/true".to_vec()).expect("program"),
        vec![Argument::new(b"first".to_vec()).expect("argument")],
        execution_limits(),
    )
    .expect("first request");
    let changed = Execute::new(
        operation_id,
        InstanceId::new([2; 16]).expect("instance ID"),
        Program::new(b"/usr/bin/true".to_vec()).expect("program"),
        vec![Argument::new(b"second".to_vec()).expect("argument")],
        execution_limits(),
    )
    .expect("changed request");

    machine.execute(first).expect("first execution succeeds");
    let failure = machine
        .execute(changed)
        .expect_err("different bytes cannot reuse the operation identity");

    assert_eq!(failure.kind(), FailureKind::OperationConflict);
}

#[test]
fn operation_receipt_limit_fails_closed_but_reserves_stop_capacity() {
    let mut machine = Machine::with_platform(DeterministicPlatform::healthy());
    machine.launch(launch()).expect("launch is ready");

    for sequence in 0..(MAX_OPERATION_RECEIPTS - 2) {
        let operation_id = (sequence as u128 + 100).to_le_bytes();
        machine
            .execute(execute(operation_id))
            .expect("receipt capacity remains");
    }

    let failure = machine
        .execute(execute([0xfe; 16]))
        .expect_err("new execution must fail before its side effect when receipts are full");
    assert_eq!(failure.kind(), FailureKind::OperationCapacityExceeded);
    assert_eq!(failure.phase(), FailurePhase::Idempotency);
    assert_eq!(failure.recovery(), Recovery::DoNotRetry);

    machine
        .stop(stop([0xff; 16]))
        .expect("one receipt slot is reserved for Stop");
}
