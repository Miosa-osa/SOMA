mod support;

use soma::{
    CleanupMethod, DestroyMachineRequest, DirectCommand, Engine, ExecuteMachineRequest,
    ExecutionLimits, InspectMachineRequest, InstanceId, LaunchMachineRequest, MachineShape,
    MachineState, ManagedFailure, ManagedStateError, OciImage, OperationId, PtyAnswer,
    PtyMachineRequest, PtyOperation, SandboxLiveness, SandboxPhase, StopMachineRequest,
    TerminalStatus,
};
use support::{Mode, TestBackend};

#[test]
fn managed_machine_executes_and_replays_a_successful_stop_without_side_effects() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::new(backend);
    let instance = InstanceId::new("22222222222222222222222222222222").expect("instance");

    let launched = engine
        .launch_machine(LaunchMachineRequest::new(
            operation('1'),
            instance.clone(),
            OciImage::parse("node:22").expect("image"),
            MachineShape::new(1, 1_024, 8_192).expect("shape"),
        ))
        .expect("launch succeeds");
    let executed = engine
        .execute_machine(ExecuteMachineRequest::new(
            operation('4'),
            instance.clone(),
            DirectCommand::new("/usr/local/bin/node", ["--version"]).expect("command"),
            ExecutionLimits::new(30_000, 16 * 1024 * 1024).expect("limits"),
        ))
        .expect("execute succeeds");
    let stop = StopMachineRequest::new(operation('5'), instance);
    let first_stop = engine.stop_machine(stop.clone()).expect("stop succeeds");
    let replayed_stop = engine.stop_machine(stop).expect("stop replay succeeds");

    assert_eq!(launched.receipt().terminal_status(), &TerminalStatus::Ready);
    assert_eq!(
        executed.receipt().terminal_status(),
        &TerminalStatus::Exited { code: 0 }
    );
    assert_eq!(first_stop, replayed_stop);
    assert_eq!(
        first_stop.receipt().terminal_status(),
        &TerminalStatus::Stopped
    );
    assert_eq!(
        first_stop.receipt().cleanup().method(),
        CleanupMethod::Graceful
    );
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "execute", "cleanup"]
    );
}

#[test]
fn inspect_returns_bounded_typed_state_with_a_receipt() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::new(backend);
    let instance = launch(&mut engine);

    let inspection = engine
        .inspect_machine(InspectMachineRequest::new(operation('4'), instance))
        .expect("inspect succeeds");

    assert_eq!(inspection.state(), MachineState::Ready);
    assert_eq!(
        inspection.receipt().terminal_status(),
        &TerminalStatus::Inspected {
            state: MachineState::Ready
        }
    );
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "inspect"]
    );
}

#[test]
fn terminal_session_survives_across_bounded_managed_calls() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::new(backend);
    let instance = launch(&mut engine);

    let opened = engine
        .pty_machine(PtyMachineRequest::new(
            operation('4'),
            instance.clone(),
            PtyOperation::Open {
                columns: 100,
                rows: 30,
            },
        ))
        .expect("terminal opens");
    let written = engine
        .pty_machine(PtyMachineRequest::new(
            operation('5'),
            instance,
            PtyOperation::Write {
                bytes: b"echo soma\n".to_vec(),
            },
        ))
        .expect("terminal remains reachable");

    assert_eq!(
        opened.answer(),
        &PtyAnswer::Opened {
            columns: 100,
            rows: 30,
        }
    );
    assert_eq!(written.answer(), &PtyAnswer::Wrote { bytes: 10 });
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "pty", "pty"]
    );
}

#[test]
fn listing_reports_phase_and_liveness_as_separate_facts() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::new(backend);
    let instance = launch(&mut engine);

    let entries = engine.list_machines().expect("listing succeeds");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].instance_id(), &instance);
    assert_eq!(entries[0].phase(), SandboxPhase::Active);
    assert_eq!(entries[0].liveness(), SandboxLiveness::Live);
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "liveness"]
    );
}

#[test]
fn destroy_replays_exactly_and_conflicts_with_stop() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::new(backend);
    let instance = launch(&mut engine);
    let request = DestroyMachineRequest::new(operation('5'), instance.clone());

    let first = engine.destroy_machine(request.clone()).expect("destroy");
    let replay = engine.destroy_machine(request).expect("destroy replay");
    let conflict = engine
        .stop_machine(StopMachineRequest::new(operation('5'), instance))
        .expect_err("stop conflicts with destroy");

    assert_eq!(first, replay);
    assert_eq!(
        first.receipt().terminal_status(),
        &TerminalStatus::Destroyed
    );
    assert_eq!(first.receipt().cleanup().method(), CleanupMethod::Forced);
    assert_eq!(
        conflict,
        ManagedFailure::State(ManagedStateError::OperationConflict)
    );
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "cleanup"]
    );
}

#[test]
fn stop_and_destroy_have_distinct_request_fingerprints() {
    let (stop_backend, _) = TestBackend::new(Mode::Happy);
    let (destroy_backend, _) = TestBackend::new(Mode::Happy);
    let mut stop_engine = Engine::new(stop_backend);
    let mut destroy_engine = Engine::new(destroy_backend);
    let stopped_instance = launch(&mut stop_engine);
    let destroyed_instance = launch(&mut destroy_engine);
    let operation_id = operation('5');

    let stopped = stop_engine
        .stop_machine(StopMachineRequest::new(
            operation_id.clone(),
            stopped_instance,
        ))
        .expect("stop");
    let destroyed = destroy_engine
        .destroy_machine(DestroyMachineRequest::new(operation_id, destroyed_instance))
        .expect("destroy");

    assert_ne!(
        stopped.receipt().request_fingerprint(),
        destroyed.receipt().request_fingerprint()
    );
}

#[test]
fn stop_receipt_discloses_a_successful_forced_fallback() {
    let (backend, _) = TestBackend::new(Mode::GracefulFallback);
    let mut engine = Engine::new(backend);
    let instance = launch(&mut engine);

    let stopped = engine
        .stop_machine(StopMachineRequest::new(operation('5'), instance))
        .expect("forced fallback still releases the machine");

    assert_eq!(
        stopped.receipt().cleanup().method(),
        CleanupMethod::GracefulThenForced
    );
    assert_eq!(
        stopped.receipt().terminal_status(),
        &TerminalStatus::Stopped
    );
}

#[test]
fn managed_timeout_cleans_and_invalidates_the_machine() {
    let (backend, calls) = TestBackend::new(Mode::Timeout);
    let mut engine = Engine::new(backend);
    let instance = launch(&mut engine);
    let command = || {
        ExecuteMachineRequest::new(
            operation('4'),
            instance.clone(),
            DirectCommand::new("/usr/local/bin/node", ["--version"]).expect("command"),
            ExecutionLimits::new(30_000, 16 * 1024 * 1024).expect("limits"),
        )
    };

    let timeout = engine
        .execute_machine(command())
        .expect_err("command times out");
    let replay_attempt = engine
        .execute_machine(command())
        .expect_err("machine is invalidated");

    assert!(matches!(timeout, ManagedFailure::Operation(_)));
    assert!(matches!(
        replay_attempt,
        ManagedFailure::ReplayUnavailable(_)
    ));
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "execute", "cleanup"]
    );
}

#[test]
fn managed_signal_cleans_and_invalidates_the_machine() {
    let (backend, calls) = TestBackend::new(Mode::Signaled);
    let mut engine = Engine::new(backend);
    let instance = launch(&mut engine);
    let command = || {
        ExecuteMachineRequest::new(
            operation('4'),
            instance.clone(),
            DirectCommand::new("/usr/local/bin/node", ["--version"]).expect("command"),
            ExecutionLimits::new(30_000, 16 * 1024 * 1024).expect("limits"),
        )
    };

    let signaled = engine
        .execute_machine(command())
        .expect_err("signal termination invalidates the machine");
    let replay_attempt = engine
        .execute_machine(command())
        .expect_err("invalidated machine cannot execute again");

    assert!(matches!(signaled, ManagedFailure::Operation(_)));
    assert!(matches!(
        replay_attempt,
        ManagedFailure::ReplayUnavailable(_)
    ));
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "execute", "cleanup"]
    );
}

fn launch(engine: &mut Engine<TestBackend>) -> InstanceId {
    let instance = InstanceId::new("22222222222222222222222222222222").expect("instance");
    engine
        .launch_machine(LaunchMachineRequest::new(
            operation('1'),
            instance.clone(),
            OciImage::parse("node:22").expect("image"),
            MachineShape::new(1, 1_024, 8_192).expect("shape"),
        ))
        .expect("launch");
    instance
}

fn operation(digit: char) -> OperationId {
    OperationId::new(digit.to_string().repeat(32)).expect("operation")
}
