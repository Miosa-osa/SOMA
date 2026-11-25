//! Releasing terminal records (ADR 0055): what is released, what is kept, and what a released
//! identity reads as afterwards.

use soma::{
    DestroyMachineRequest, Engine, InspectMachineRequest, InstanceId, MachineRelease,
    ManagedFailure, ManagedStateError, MemoryStateStore, StateRecord, StateStore,
};

use crate::{
    fixtures::{instance, launch_request, operation},
    support::{Mode, TestBackend},
};

fn other_instance() -> InstanceId {
    InstanceId::new("44444444444444444444444444444444").expect("instance")
}

fn destroy(digit: char, instance_id: InstanceId) -> DestroyMachineRequest {
    DestroyMachineRequest::new(operation(digit), instance_id)
}

fn not_found<T: std::fmt::Debug>(result: &Result<T, ManagedFailure>) -> bool {
    matches!(
        result,
        Err(ManagedFailure::State(ManagedStateError::MachineNotFound))
    )
}

#[test]
fn only_a_terminal_record_is_released_and_a_live_one_is_kept_whole() {
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut store = MemoryStateStore::new();
    let mut engine = Engine::with_state_store(backend, store.clone());
    engine
        .launch_machine(launch_request(operation('1'), instance()))
        .expect("launch the one that ends");
    engine
        .launch_machine(launch_request(operation('2'), other_instance()))
        .expect("launch the one that lives");
    engine
        .destroy_machine(destroy('3', instance()))
        .expect("destroy");

    let census = engine.terminal_census().expect("census");
    assert_eq!(census.records(), 2);
    assert_eq!(census.terminal(), [instance()]);
    let live = store.load(&other_instance()).expect("load").expect("live");

    assert_eq!(
        engine.release_machine(&other_instance()).expect("judge"),
        MachineRelease::Kept
    );
    assert_eq!(
        store.load(&other_instance()).expect("load").expect("kept"),
        live,
        "a kept record is not rewritten"
    );
    assert_eq!(
        engine.release_machine(&instance()).expect("release"),
        MachineRelease::Released
    );
    assert_eq!(
        engine.release_machine(&instance()).expect("repeat"),
        MachineRelease::Absent
    );
    assert_eq!(store.list().expect("list"), [other_instance()]);
    let listed = engine.list_machines().expect("listing");
    assert_eq!(listed.len(), 1, "recovery still sees exactly the live one");
    assert_eq!(listed[0].instance_id(), &other_instance());
}

#[test]
fn a_destroy_replays_until_its_record_is_released_and_then_reads_as_never_created() {
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::with_state_store(backend, MemoryStateStore::new());
    engine
        .launch_machine(launch_request(operation('1'), instance()))
        .expect("launch");
    let first = engine
        .destroy_machine(destroy('2', instance()))
        .expect("destroy");
    let replay = engine
        .destroy_machine(destroy('2', instance()))
        .expect("an exact repeat replays while the record is retained");
    assert_eq!(first, replay);

    engine.release_machine(&instance()).expect("release");

    assert!(not_found(&engine.destroy_machine(destroy('2', instance()))));
    assert!(not_found(&engine.destroy_machine(destroy('4', instance()))));
    assert!(not_found(&engine.inspect_machine(
        InspectMachineRequest::new(operation('5'), instance())
    )));
    engine
        .launch_machine(launch_request(operation('6'), instance()))
        .expect("a released identity may be created again");
}

#[test]
fn a_failed_launch_leaves_a_terminal_record_that_is_released() {
    let (backend, _) = TestBackend::new(Mode::LaunchFailure);
    let mut store = MemoryStateStore::new();
    let mut engine = Engine::with_state_store(backend, store.clone());
    assert!(matches!(
        engine.launch_machine(launch_request(operation('1'), instance())),
        Err(ManagedFailure::Operation(_))
    ));

    assert_eq!(
        engine.terminal_census().expect("census").terminal(),
        [instance()]
    );
    assert_eq!(
        engine.release_machine(&instance()).expect("release"),
        MachineRelease::Released
    );
    assert!(store.list().expect("list").is_empty());
}

#[test]
fn an_undecodable_record_is_counted_and_never_released() {
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut store = MemoryStateStore::new();
    let mut engine = Engine::with_state_store(backend, store.clone());
    let garbage = StateRecord::from_bytes(b"{\"schema_version\":2}".to_vec()).expect("record");
    store
        .create(&other_instance(), garbage.clone())
        .expect("plant a record nothing can decode");

    let census = engine.terminal_census().expect("census");
    assert_eq!(census.records(), 1);
    assert!(census.terminal().is_empty());
    assert!(matches!(
        engine.release_machine(&other_instance()),
        Err(ManagedFailure::StateStore(_))
    ));
    assert_eq!(
        store.load(&other_instance()).expect("load").expect("kept"),
        soma::StoredState::new(soma::StateRevision::INITIAL, garbage)
    );
}

/// The store-level leak behind SOMA#127: without a release, every create and destroy left one
/// record behind for good. With one, a long run of them leaves the store holding nothing.
#[test]
fn a_long_run_of_create_and_destroy_with_release_leaves_the_store_empty() {
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut store = MemoryStateStore::new();
    let mut engine = Engine::with_state_store(backend, store.clone());
    for cycle in 1..=1_000_u32 {
        let instance_id = InstanceId::new(format!("{cycle:032x}")).expect("instance");
        engine
            .launch_machine(launch_request(operation('1'), instance_id.clone()))
            .expect("launch");
        engine
            .destroy_machine(destroy('2', instance_id))
            .expect("destroy");
        for terminal in engine.terminal_census().expect("census").into_terminal() {
            engine.release_machine(&terminal).expect("release");
        }
        assert!(store.list().expect("list").is_empty(), "cycle {cycle}");
    }
}

/// SOMA#223: a destroyed Instance's identity is free for a new create at once, not only after
/// the retention sweep released its record.
#[test]
fn a_destroyed_identity_may_be_created_again_before_its_record_is_released() {
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::with_state_store(backend, MemoryStateStore::new());
    engine
        .launch_machine(launch_request(operation('1'), instance()))
        .expect("launch");
    engine
        .destroy_machine(destroy('2', instance()))
        .expect("destroy");

    engine
        .launch_machine(launch_request(operation('3'), instance()))
        .expect("a new create reuses the destroyed identity");
    assert!(
        engine
            .terminal_census()
            .expect("census")
            .terminal()
            .is_empty(),
        "the new create replaced the terminal record"
    );
}

/// SOMA#223: a create the backend refused leaves no record that blocks the next create.
#[test]
fn a_failed_launch_does_not_block_the_next_create_of_the_same_identity() {
    let (failing, _) = TestBackend::new(Mode::LaunchFailure);
    let store = MemoryStateStore::new();
    let mut refused = Engine::with_state_store(failing, store.clone());
    assert!(matches!(
        refused.launch_machine(launch_request(operation('1'), instance())),
        Err(ManagedFailure::Operation(_))
    ));

    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::with_state_store(backend, store);
    engine
        .launch_machine(launch_request(operation('2'), instance()))
        .expect("a create after a failed one reuses the identity");
}

/// An exact repeat of the operation that ended the record still reads that answer, so a retry
/// after a lost reply cannot start a second machine.
#[test]
fn an_exact_repeat_of_a_failed_launch_still_replays_instead_of_launching() {
    let (failing, _) = TestBackend::new(Mode::LaunchFailure);
    let store = MemoryStateStore::new();
    let mut refused = Engine::with_state_store(failing, store.clone());
    assert!(
        refused
            .launch_machine(launch_request(operation('1'), instance()))
            .is_err()
    );

    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::with_state_store(backend, store);
    assert!(matches!(
        engine.launch_machine(launch_request(operation('1'), instance())),
        Err(ManagedFailure::ReplayUnavailable(_))
    ));
}
