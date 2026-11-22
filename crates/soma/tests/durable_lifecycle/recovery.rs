use soma::{Engine, ManagedFailure, ManagedStateError, StopMachineRequest};

use crate::{
    fixtures::{FailingCasStore, execute_request, instance, launch_request, operation},
    support::{Mode, TestBackend},
};

#[test]
fn interrupted_launch_is_rolled_back_before_exact_retry() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let store = FailingCasStore::new([1]);
    let instance = instance();
    let request = launch_request(operation('1'), instance);
    let mut first = Engine::with_state_store(backend.clone(), store.clone());

    assert!(matches!(
        first.launch_machine(request.clone()),
        Err(ManagedFailure::Operation(_))
    ));
    drop(first);

    let mut restarted = Engine::with_state_store(backend, store);
    restarted
        .launch_machine(request)
        .expect("retry cleans the uncertain launch before relaunching");
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "resolve", "cleanup", "launch"]
    );
}

#[test]
fn interrupted_execute_is_cleaned_and_never_run_twice() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let store = FailingCasStore::new([3]);
    let instance = instance();
    let mut first = Engine::with_state_store(backend.clone(), store.clone());
    first
        .launch_machine(launch_request(operation('1'), instance.clone()))
        .expect("launch");
    let execute = execute_request(operation('4'), instance.clone());
    assert!(matches!(
        first.execute_machine(execute.clone()),
        Err(ManagedFailure::Operation(_))
    ));
    drop(first);

    let mut restarted = Engine::with_state_store(backend.clone(), store.clone());
    assert!(matches!(
        restarted.execute_machine(execute.clone()),
        Err(ManagedFailure::Operation(_))
    ));
    assert!(matches!(
        restarted.execute_machine(execute),
        Err(ManagedFailure::ReplayUnavailable(_))
    ));
    assert_eq!(
        restarted
            .execute_machine(execute_request(operation('5'), instance))
            .expect_err("cleaned machine remains terminal"),
        ManagedFailure::State(ManagedStateError::MachineStopped)
    );
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "execute", "cleanup"]
    );
}

#[test]
fn interrupted_stop_resumes_cleanup_and_then_replays_exactly() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let store = FailingCasStore::new([3]);
    let instance = instance();
    let stop = StopMachineRequest::new(operation('5'), instance.clone());
    let mut first = Engine::with_state_store(backend.clone(), store.clone());
    first
        .launch_machine(launch_request(operation('1'), instance))
        .expect("launch");
    assert!(matches!(
        first.stop_machine(stop.clone()),
        Err(ManagedFailure::Operation(_))
    ));
    drop(first);

    let mut restarted = Engine::with_state_store(backend.clone(), store.clone());
    let completed = restarted
        .stop_machine(stop.clone())
        .expect("cleanup resumes");
    drop(restarted);
    let mut replay = Engine::with_state_store(backend, store);
    assert_eq!(completed, replay.stop_machine(stop).expect("exact replay"));
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "cleanup", "cleanup"]
    );
}

#[test]
fn replacement_broker_waits_for_publication_and_only_restores_the_authorized_capture() {
    use soma::{GenerationId, MachineName, StateStoreFailureKind};

    let (backend, calls) = TestBackend::new(Mode::Happy);
    let store = FailingCasStore::new([1]);
    let instance = instance();
    let owner = MachineName::parse("tenant-a").expect("owner");
    let other = MachineName::parse("tenant-b").expect("other owner");
    let generation = GenerationId::new(format!("sha256:{}", "3".repeat(64))).expect("generation");
    let digest = GenerationId::new(format!("sha256:{}", "4".repeat(64))).expect("receipt");
    let checkpoint = operation('a');
    let restore = launch_request(operation('b'), instance.clone()).with_name(owner.clone());
    let mut first = Engine::with_state_store(backend.clone(), store.clone());

    let install = |engine: &mut Engine<_, _>| {
        engine.install_recovery_source(&instance, &owner, &checkpoint, &generation, &digest, || {
            Ok::<_, ()>(())
        })
    };
    assert_eq!(
        install(&mut first),
        Err(ManagedFailure::StateStore(
            StateStoreFailureKind::Unavailable
        ))
    );
    assert_eq!(first.restore_owner(&instance).expect("owner"), None);
    assert!(first.list_machines().expect("census").is_empty());
    assert_eq!(
        first.launch_machine(restore.clone()).expect_err("reserved"),
        ManagedFailure::State(ManagedStateError::OperationConflict)
    );
    assert_eq!(
        first
            .restore_machine(restore.clone(), &checkpoint)
            .expect_err("unconfirmed"),
        ManagedFailure::State(ManagedStateError::OperationConflict),
    );
    drop(first);

    let mut restarted = Engine::with_state_store(backend, store);
    assert_eq!(
        restarted.install_recovery_source(
            &instance,
            &other,
            &checkpoint,
            &generation,
            &digest,
            || -> Result<(), ()> {
                panic!("wrong owner must not publish");
            }
        ),
        Err(ManagedFailure::State(ManagedStateError::OperationConflict)),
    );
    assert_eq!(install(&mut restarted), Ok(Ok(())));
    assert_eq!(install(&mut restarted), Ok(Ok(())));
    assert_eq!(
        restarted.restore_owner(&instance).expect("owner"),
        Some(owner)
    );

    assert!(
        !restarted
            .permits_checkpoint_transfer(&instance, &checkpoint, &generation, &generation)
            .expect("binding")
    );
    assert_eq!(
        restarted
            .restore_machine(restore.clone(), &operation('c'))
            .expect_err("wrong checkpoint"),
        ManagedFailure::State(ManagedStateError::OperationConflict),
    );
    assert_eq!(
        restarted
            .restore_machine(restore.clone().with_name(other), &checkpoint)
            .expect_err("wrong owner"),
        ManagedFailure::State(ManagedStateError::OperationConflict),
    );

    let ready = restarted
        .restore_machine(restore.clone(), &checkpoint)
        .expect("restore");
    assert_eq!(
        restarted
            .restore_machine(restore, &checkpoint)
            .expect("replay"),
        ready
    );
    assert_eq!(
        calls
            .lock()
            .expect("call log")
            .iter()
            .filter(|call| **call == "launch_checkpoint")
            .count(),
        1,
    );
}
