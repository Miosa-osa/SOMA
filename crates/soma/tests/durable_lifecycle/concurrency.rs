use std::thread;

use soma::{Engine, ManagedFailure, ManagedStateError, MemoryStateStore, StopMachineRequest};

use crate::{
    fixtures::{execute_request, instance, launch_request, operation},
    support::{Mode, TestBackend},
};

#[test]
fn concurrent_execute_transition_allows_only_one_guest_command() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let (backend, gate) = backend.with_execute_gate();
    let store = MemoryStateStore::new();
    let instance = instance();
    let mut launcher = Engine::with_state_store(backend.clone(), store.clone());
    launcher
        .launch_machine(launch_request(operation('1'), instance.clone()))
        .expect("launch");
    let worker_backend = backend.clone();
    let worker_store = store.clone();
    let worker_instance = instance.clone();
    let worker = thread::spawn(move || {
        let mut engine = Engine::with_state_store(worker_backend, worker_store);
        engine.execute_machine(execute_request(operation('4'), worker_instance))
    });
    gate.wait_until_started();

    let mut contender = Engine::with_state_store(backend, store);
    assert!(matches!(
        contender.execute_machine(execute_request(operation('5'), instance)),
        Err(ManagedFailure::Operation(_))
    ));
    gate.release();
    assert!(matches!(
        worker.join().expect("worker joins"),
        Err(ManagedFailure::Operation(_))
    ));
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "execute", "cleanup"]
    );
}

#[test]
fn concurrent_termination_transition_rejects_a_different_owner() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let (backend, gate) = backend.with_cleanup_gate();
    let store = MemoryStateStore::new();
    let instance = instance();
    let mut launcher = Engine::with_state_store(backend.clone(), store.clone());
    launcher
        .launch_machine(launch_request(operation('1'), instance.clone()))
        .expect("launch");
    let worker_backend = backend.clone();
    let worker_store = store.clone();
    let worker_instance = instance.clone();
    let worker = thread::spawn(move || {
        let mut engine = Engine::with_state_store(worker_backend, worker_store);
        engine.stop_machine(StopMachineRequest::new(operation('5'), worker_instance))
    });
    gate.wait_until_started();

    let mut contender = Engine::with_state_store(backend, store);
    assert_eq!(
        contender
            .destroy_machine(soma::DestroyMachineRequest::new(operation('6'), instance))
            .expect_err("different termination cannot acquire the transition"),
        ManagedFailure::State(ManagedStateError::OperationConflict)
    );
    gate.release();
    worker
        .join()
        .expect("worker joins")
        .expect("stop completes");
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "cleanup"]
    );
}
