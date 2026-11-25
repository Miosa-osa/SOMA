use std::{thread, time::Duration};

use soma::{
    Engine, FileMachineRequest, FileOperation, InstanceId, ManagedFailure, ManagedStateError,
    MemoryStateStore, StopMachineRequest,
};

use crate::{
    fixtures::{execute_request, instance, launch_request, operation},
    support::{Mode, TestBackend},
};

/// Agents run several commands at once on one sandbox. A second command waits its turn rather
/// than finding the first one's Executing record and taking it for a crash (SOMA#243).
#[test]
fn a_second_command_waits_for_the_running_one_and_both_succeed() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let (backend, gate) = backend.with_execute_gate();
    let store = MemoryStateStore::new();
    let instance = instance();
    let mut launcher = Engine::with_state_store(backend.clone(), store.clone());
    launcher
        .launch_machine(launch_request(operation('1'), instance.clone()))
        .expect("launch");
    let worker = spawn_execute(&backend, &store, &instance, '4');
    gate.wait_until_started();

    let contender = spawn_execute(&backend, &store, &instance, '5');
    let file = {
        let (backend, store, instance) = (backend.clone(), store.clone(), instance.clone());
        thread::spawn(move || {
            Engine::with_state_store(backend, store).file_machine(FileMachineRequest::new(
                operation('6'),
                instance,
                FileOperation::Read {
                    path: b"/etc/hostname".to_vec(),
                },
            ))
        })
    };
    thread::sleep(Duration::from_millis(300));
    assert!(
        !contender.is_finished() && !file.is_finished(),
        "an operation on a machine running a command must wait for it, not answer at once"
    );
    gate.release();
    worker.join().expect("worker joins").expect("first command");
    contender
        .join()
        .expect("contender joins")
        .expect("second command");
    file.join().expect("file joins").expect("file operation");
    let calls = calls.lock().expect("call log poisoned").clone();
    assert!(
        !calls.contains(&"cleanup"),
        "the machine was released: {calls:?}"
    );
    assert_eq!(calls.iter().filter(|call| **call == "execute").count(), 2);
}

fn spawn_execute(
    backend: &TestBackend,
    store: &MemoryStateStore,
    instance: &InstanceId,
    digit: char,
) -> thread::JoinHandle<Result<soma::MachineExecution, ManagedFailure>> {
    let (backend, store, instance) = (backend.clone(), store.clone(), instance.clone());
    thread::spawn(move || {
        Engine::with_state_store(backend, store)
            .execute_machine(execute_request(operation(digit), instance))
    })
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
