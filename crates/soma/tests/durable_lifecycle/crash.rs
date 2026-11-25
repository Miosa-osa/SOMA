//! A machine whose worker died, a launch its host restarted under, and a failed launch can all be
//! let go of and created again under the same identity (SOMA#220).

use soma::{Engine, ManagedFailure, ManagedStateError, StopMachineRequest};

use crate::{
    fixtures::{FailingCasStore, instance, launch_request, operation},
    support::{Mode, TestBackend},
};

/// SOMA#220: a launch left launching (its host restarted under it) can be destroyed, and the
/// identity can then be created again.
#[test]
fn a_launch_left_launching_can_be_destroyed_and_created_again() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let store = FailingCasStore::new([1]);
    let mut first = Engine::with_state_store(backend.clone(), store.clone());
    assert!(
        first
            .launch_machine(launch_request(operation('1'), instance()))
            .is_err()
    );
    drop(first);

    let mut restarted = Engine::with_state_store(backend, store);
    let destroyed = restarted
        .destroy_machine(soma::DestroyMachineRequest::new(operation('2'), instance()))
        .expect("the stuck launch is rolled back and destroyed");
    assert_eq!(
        destroyed.receipt().terminal_status(),
        &soma::TerminalStatus::Destroyed
    );
    assert!(
        calls.lock().expect("call log").contains(&"cleanup"),
        "the half-built machine was released"
    );
    let replay = restarted
        .destroy_machine(soma::DestroyMachineRequest::new(operation('2'), instance()))
        .expect("the same destroy replays");
    assert_eq!(replay.receipt(), destroyed.receipt());
    restarted
        .launch_machine(launch_request(operation('3'), instance()))
        .expect("the identity is free again");
}

/// SOMA#220: a create that the backend refused leaves a record that a destroy now releases.
#[test]
fn a_failed_launch_can_be_destroyed() {
    let (backend, _) = TestBackend::new(Mode::LaunchFailure);
    let mut engine = Engine::with_state_store(backend, soma::MemoryStateStore::new());
    assert!(
        engine
            .launch_machine(launch_request(operation('1'), instance()))
            .is_err()
    );
    engine
        .destroy_machine(soma::DestroyMachineRequest::new(operation('2'), instance()))
        .expect("a failed launch's record is destroyed");
}

/// SOMA#220: the worker of a running machine died; a create of the same identity boots it again.
#[test]
fn a_crashed_machine_is_booted_again_by_a_create_of_its_identity() {
    let store = soma::MemoryStateStore::new();
    let (live, _) = TestBackend::new(Mode::Happy);
    Engine::with_state_store(live, store.clone())
        .launch_machine(launch_request(operation('1'), instance()))
        .expect("launch");

    let (crashed, calls) = TestBackend::new(Mode::Crashed);
    let mut engine = Engine::with_state_store(crashed, store);
    engine
        .launch_machine(launch_request(operation('2'), instance()))
        .expect("a machine reported gone is booted again under its identity");
    let log = calls.lock().expect("call log").clone();
    let cleanup = log.iter().position(|call| *call == "cleanup");
    let launch = log.iter().rposition(|call| *call == "launch");
    assert!(
        cleanup < launch,
        "what was left is released before the new boot: {log:?}"
    );
}

/// A machine the backend reports live keeps its identity: a second create is refused.
#[test]
fn a_live_machine_is_never_replaced_by_a_create_of_its_identity() {
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::with_state_store(backend, soma::MemoryStateStore::new());
    engine
        .launch_machine(launch_request(operation('1'), instance()))
        .expect("launch");
    assert!(matches!(
        engine.launch_machine(launch_request(operation('2'), instance())),
        Err(ManagedFailure::State(
            ManagedStateError::MachineAlreadyExists
        ))
    ));
}

/// SOMA#220: a stop of a stopped machine answers with the stop that stopped it.
#[test]
fn a_second_stop_answers_like_the_first() {
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::with_state_store(backend, soma::MemoryStateStore::new());
    engine
        .launch_machine(launch_request(operation('1'), instance()))
        .expect("launch");
    let first = engine
        .stop_machine(StopMachineRequest::new(operation('2'), instance()))
        .expect("stop");
    let second = engine
        .stop_machine(StopMachineRequest::new(operation('3'), instance()))
        .expect("a second stop is not a conflict");
    assert_eq!(first.receipt(), second.receipt());
}
