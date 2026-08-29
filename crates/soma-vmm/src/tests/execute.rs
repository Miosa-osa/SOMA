use crate::{CleanupEvidence, ExitStatus, FailureKind, FailurePhase, Machine, Recovery};

use super::{
    fixtures::{execute, execute_with_output_limit, launch},
    platform::{DeterministicPlatform, TestStage},
};

#[test]
fn execute_is_rejected_before_the_instance_is_ready() {
    let mut machine = Machine::with_platform(DeterministicPlatform::healthy());

    let failure = machine
        .execute(execute([4; 16]))
        .expect_err("execute cannot bypass Launch readiness");

    assert_eq!(failure.kind(), FailureKind::InvalidLifecycle);
    assert_eq!(failure.phase(), FailurePhase::Lifecycle);
    assert_eq!(failure.cleanup(), CleanupEvidence::NotRequired);
}

#[test]
fn execute_returns_authenticated_terminal_output_for_the_ready_instance() {
    let mut machine = Machine::with_platform(DeterministicPlatform::healthy());
    machine.launch(launch()).expect("launch is ready");

    let executed = machine
        .execute(execute([4; 16]))
        .expect("ready instance executes through its authenticated session");

    assert_eq!(executed.status(), ExitStatus::Code(0));
    assert_eq!(executed.stdout(), b"execution-1");
    assert_eq!(executed.stderr(), b"");
}

#[test]
fn execute_platform_failure_is_typed_and_replayed() {
    let mut machine =
        Machine::with_platform(DeterministicPlatform::failing_at(TestStage::UserExecute));
    machine.launch(launch()).expect("launch is ready");
    let request = execute([8; 16]);

    let first = machine
        .execute(request.clone())
        .expect_err("platform execution failure is terminal for the operation");
    let replay = machine
        .execute(request)
        .expect_err("the failure receipt is idempotently replayed");

    assert_eq!(replay, first);
    assert_eq!(first.kind(), FailureKind::ExecuteFailed);
    assert_eq!(first.phase(), FailurePhase::Execute);
    assert_eq!(first.recovery(), Recovery::ReplaceMachine);
    assert_eq!(first.cleanup(), CleanupEvidence::NotRequired);
}

#[test]
fn execute_enforces_the_output_limit_when_the_platform_misbehaves() {
    let mut machine = Machine::with_platform(DeterministicPlatform::oversized_output());
    machine.launch(launch()).expect("launch is ready");

    let executed = machine
        .execute(execute_with_output_limit([7; 16], 10))
        .expect("the seam contains oversized platform output");

    assert_eq!(executed.status(), ExitStatus::OutputLimit);
    assert_eq!(executed.stdout(), b"abcdefgh");
    assert_eq!(executed.stderr(), b"WX");
    assert_eq!(executed.stdout().len() + executed.stderr().len(), 10);
}

#[test]
fn byte_equivalent_execute_replay_returns_the_original_receipt() {
    let mut machine = Machine::with_platform(DeterministicPlatform::healthy());
    machine.launch(launch()).expect("launch is ready");
    let request = execute([4; 16]);
    let first = machine
        .execute(request.clone())
        .expect("first execution succeeds");

    let replay = machine
        .execute(request)
        .expect("exact execution replay is idempotent");

    assert_eq!(replay, first);
    assert_eq!(replay.stdout(), b"execution-1");
}
