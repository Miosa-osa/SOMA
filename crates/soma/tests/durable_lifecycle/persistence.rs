use soma::{
    Engine, InspectMachineRequest, ManagedFailure, ManagedStateError, MemoryStateStore,
    OperationId, StateRecord, StateStoreFailureKind, StopMachineRequest,
};

use crate::{
    fixtures::{
        StaticStore, execute_request, instance, launch_request, operation, valid_active_record,
    },
    support::{Mode, TestBackend},
};

#[test]
fn managed_state_survives_engine_restarts_and_replays_without_side_effects() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let store = MemoryStateStore::new();
    let instance = instance();
    let mut launch_engine = Engine::with_state_store(backend.clone(), store.clone());
    launch_engine
        .launch_machine(launch_request(operation('1'), instance.clone()))
        .expect("launch");
    drop(launch_engine);

    let mut command_engine = Engine::with_state_store(backend.clone(), store.clone());
    command_engine
        .inspect_machine(InspectMachineRequest::new(operation('3'), instance.clone()))
        .expect("inspect after restart");
    command_engine
        .execute_machine(execute_request(operation('4'), instance.clone()))
        .expect("execute after restart");
    drop(command_engine);

    let mut replay_engine = Engine::with_state_store(backend.clone(), store.clone());
    assert!(matches!(
        replay_engine.execute_machine(execute_request(operation('4'), instance.clone())),
        Err(ManagedFailure::ReplayUnavailable(_))
    ));
    let stop = StopMachineRequest::new(operation('5'), instance.clone());
    let first = replay_engine.stop_machine(stop.clone()).expect("stop");
    drop(replay_engine);

    let mut terminal_engine = Engine::with_state_store(backend, store);
    let replay = terminal_engine
        .stop_machine(stop)
        .expect("exact stop replay");
    assert_eq!(first, replay);
    assert_eq!(
        *calls.lock().expect("call log poisoned"),
        ["resolve", "launch", "inspect", "execute", "cleanup"]
    );
}

#[test]
fn corrupt_mismatched_and_unsupported_documents_fail_before_backend_calls() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let instance = instance();
    let valid = valid_active_record();
    let mismatched = StateRecord::from_bytes(
        String::from_utf8(valid.as_bytes().to_vec())
            .expect("state JSON")
            .replace(
                "22222222222222222222222222222222",
                "33333333333333333333333333333333",
            )
            .into_bytes(),
    )
    .expect("bounded mismatch");
    let mut unknown = valid.as_bytes().to_vec();
    unknown.pop();
    unknown.extend_from_slice(br#","unexpected":true}"#);
    for (record, expected) in [
        (
            StateRecord::from_bytes(br#"{"schema_version":2}"#.to_vec()).expect("record"),
            StateStoreFailureKind::Corrupt,
        ),
        (
            StateRecord::from_bytes(br#"{"schema_version":999}"#.to_vec()).expect("record"),
            StateStoreFailureKind::UnsupportedVersion,
        ),
        (mismatched, StateStoreFailureKind::Corrupt),
        (
            StateRecord::from_bytes(unknown).expect("bounded unknown field"),
            StateStoreFailureKind::Corrupt,
        ),
    ] {
        let store = StaticStore { record };
        let mut engine = Engine::with_state_store(backend.clone(), store);
        assert_eq!(
            engine
                .inspect_machine(InspectMachineRequest::new(operation('3'), instance.clone()))
                .expect_err("document must fail closed"),
            ManagedFailure::StateStore(expected)
        );
    }
    assert!(calls.lock().expect("call log poisoned").is_empty());
}

#[test]
fn replay_tombstone_capacity_rejects_before_a_new_guest_command() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::new(backend);
    let instance = instance();
    engine
        .launch_machine(launch_request(operation('f'), instance.clone()))
        .expect("launch");
    for sequence in 1_u128..=1_024 {
        engine
            .execute_machine(execute_request(
                OperationId::new(format!("{sequence:032x}")).expect("operation"),
                instance.clone(),
            ))
            .expect("bounded execution ledger");
    }
    let failure = engine
        .execute_machine(execute_request(
            OperationId::new(format!("{:032x}", 1_025_u128)).expect("operation"),
            instance,
        ))
        .expect_err("full replay ledger requires replacement");

    assert_eq!(
        failure,
        ManagedFailure::State(ManagedStateError::ReplayCapacityReached)
    );
    assert_eq!(
        calls
            .lock()
            .expect("call log poisoned")
            .iter()
            .filter(|call| **call == "execute")
            .count(),
        1_024
    );
}
