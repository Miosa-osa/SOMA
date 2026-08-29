mod support;

use soma::{
    BackendFailureKind, Capabilities, DirectCommand, Engine, ExecutionLimits, FailurePhase,
    InstanceId, MachineShape, OciImage, OperationId, RunFailureKind, RunRequest, TerminalStatus,
};
use support::{Mode, TestBackend, run_request, run_request_with_output_limit};

#[test]
fn command_failure_preserves_evidence_and_still_cleans_the_machine() {
    let (backend, calls) = TestBackend::new(Mode::CommandFailure);
    let mut engine = Engine::new(backend);

    let failure = engine.run(run_request()).expect_err("command fails");

    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "execute", "cleanup"]
    );
    assert_eq!(
        failure.kind(),
        RunFailureKind::Backend {
            phase: FailurePhase::Command,
            kind: BackendFailureKind::GuestFailure,
        }
    );
    assert_eq!(failure.receipt().terminal_status(), &TerminalStatus::Failed);
    assert!(failure.receipt().cleanup().is_complete());
}

#[test]
fn timeout_returns_bounded_output_evidence_after_cleanup() {
    let (backend, calls) = TestBackend::new(Mode::Timeout);
    let mut engine = Engine::new(backend);

    let failure = engine.run(run_request()).expect_err("command times out");

    assert_eq!(failure.kind(), RunFailureKind::TimedOut);
    assert_eq!(
        failure.receipt().terminal_status(),
        &TerminalStatus::TimedOut
    );
    assert!(failure.receipt().cleanup().is_complete());
    assert_eq!(
        failure.output().expect("bounded output retained").stdout(),
        b"v22.23.2\n"
    );
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "execute", "cleanup"]
    );
}

#[test]
fn failed_launch_rolls_back_the_preassigned_instance() {
    let (backend, calls) = TestBackend::new(Mode::LaunchFailure);
    let mut engine = Engine::new(backend);

    let failure = engine.run(run_request()).expect_err("launch fails");

    assert_eq!(
        failure.kind(),
        RunFailureKind::Backend {
            phase: FailurePhase::Launch,
            kind: BackendFailureKind::IsolationFailure,
        }
    );
    assert!(failure.receipt().cleanup().is_complete());
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "cleanup"]
    );
}

#[test]
fn mismatched_backend_identity_evidence_is_rejected_and_cleaned() {
    let (backend, calls) = TestBackend::new(Mode::CommandIdentityMismatch);
    let mut engine = Engine::new(backend);

    let failure = engine
        .run(run_request())
        .expect_err("mismatched instance must fail closed");

    assert_eq!(failure.kind(), RunFailureKind::ObservationMismatch);
    assert!(failure.receipt().cleanup().is_complete());
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "execute", "cleanup"]
    );
}

#[test]
fn non_monotonic_backend_timing_fails_closed_without_corrupting_the_receipt() {
    let (backend, _) = TestBackend::new(Mode::NonMonotonicCommand);
    let mut engine = Engine::new(backend);

    let failure = engine
        .run(run_request())
        .expect_err("non-monotonic evidence must fail closed");

    assert_eq!(failure.kind(), RunFailureKind::ObservationMismatch);
    assert!(
        failure
            .receipt()
            .milestones()
            .windows(2)
            .all(|pair| pair[0].elapsed_ns() <= pair[1].elapsed_ns())
    );
}

#[test]
fn cleanup_failure_is_terminal_and_preserves_the_completed_command() {
    let (backend, _) = TestBackend::new(Mode::CleanupFailure);
    let mut engine = Engine::new(backend);

    let failure = engine.run(run_request()).expect_err("cleanup fails");

    assert_eq!(
        failure.kind(),
        RunFailureKind::Backend {
            phase: FailurePhase::Cleanup,
            kind: BackendFailureKind::CleanupFailure,
        }
    );
    assert!(!failure.receipt().cleanup().is_complete());
    assert_eq!(
        failure
            .output()
            .expect("command output is retained")
            .stdout(),
        b"v22.23.2\n"
    );
}

#[test]
fn regressing_failure_timestamps_are_rejected_without_emitting_invalid_receipts() {
    for mode in [
        Mode::FailureTimeRegression,
        Mode::CleanupFailureTimeRegression,
    ] {
        let (backend, _) = TestBackend::new(mode);
        let mut engine = Engine::new(backend);

        let failure = engine
            .run(run_request())
            .expect_err("backend evidence regresses");

        assert_eq!(failure.kind(), RunFailureKind::ObservationMismatch);
        failure
            .receipt()
            .validate()
            .expect("facade emits only valid receipts");
    }
}

#[test]
fn combined_stdout_and_stderr_capture_cannot_exceed_one_allowance() {
    let (backend, calls) = TestBackend::new(Mode::CombinedOutputOverflow);
    let mut engine = Engine::new(backend);

    let failure = engine
        .run(run_request_with_output_limit(10))
        .expect_err("combined capture exceeds the allowance");

    assert_eq!(failure.kind(), RunFailureKind::ObservationMismatch);
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "execute", "cleanup"]
    );
}

#[test]
fn network_denial_requires_observed_enforcement() {
    let (backend, calls) = TestBackend::new(Mode::UnverifiedNetworkDenial);
    let mut engine = Engine::new(backend);
    let shape = MachineShape::new(1, 1_024, 8_192)
        .expect("shape")
        .with_capabilities(Capabilities::isolated());
    let request = RunRequest::new(
        OperationId::new("11111111111111111111111111111111").expect("operation"),
        InstanceId::new("22222222222222222222222222222222").expect("instance"),
        OciImage::parse("node:22").expect("image"),
        shape,
        DirectCommand::new("/usr/local/bin/node", ["--version"]).expect("command"),
        ExecutionLimits::new(30_000, 1_048_576).expect("limits"),
    );

    let failure = engine
        .run(request)
        .expect_err("unverified network denial must fail closed");

    assert_eq!(failure.kind(), RunFailureKind::ObservationMismatch);
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "cleanup"]
    );
}
