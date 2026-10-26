use soma::{
    Engine, GenerationId, InstanceId, ManagedFailure, MemoryStateStore, StateRecord, StateStore,
    StateStoreFailureKind,
};

use crate::{
    fixtures::{instance, launch_request, operation},
    support::{Mode, TestBackend},
};

fn other_instance() -> InstanceId {
    InstanceId::new("33333333333333333333333333333333").expect("instance")
}

fn garbage() -> StateRecord {
    StateRecord::from_bytes(b"{\"schema_version\":2,\"not\":\"a machine\"}".to_vec())
        .expect("bounded document")
}

#[test]
fn failed_launch_record_stays_readable_so_listing_survives_it() {
    // A failed launch leaves a terminal record. The store reads it back through the receipt
    // decoder, so a receipt the decoder rejects broke every listing until retention removed it.
    let (backend, _) = TestBackend::new(Mode::LaunchFailure);
    let store = MemoryStateStore::new();
    let mut engine = Engine::with_state_store(backend, store);

    assert!(matches!(
        engine.launch_machine(launch_request(operation('1'), instance())),
        Err(ManagedFailure::Operation(_))
    ));

    let entries = engine
        .list_machines()
        .expect("a failed launch's record decodes");
    assert!(entries.is_empty(), "a terminal record is not a sandbox");
}

#[test]
fn listing_fails_whole_on_an_undecodable_record_unless_the_caller_asks_for_a_report() {
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut store = MemoryStateStore::new();
    let mut engine = Engine::with_state_store(backend, store.clone());
    engine
        .launch_machine(launch_request(operation('1'), instance()))
        .expect("launch");
    store
        .create(&other_instance(), garbage())
        .expect("plant a record that cannot be decoded");

    assert!(matches!(
        engine.list_machines(),
        Err(ManagedFailure::StateStore(
            StateStoreFailureKind::InvalidRecord
                | StateStoreFailureKind::Corrupt
                | StateStoreFailureKind::UnsupportedVersion
        ))
    ));

    let listing = engine
        .list_machines_reporting_unreadable()
        .expect("the readable sandbox is still listed");
    assert_eq!(listing.entries().len(), 1);
    assert_eq!(listing.entries()[0].instance_id(), &instance());
    assert_eq!(listing.unreadable().len(), 1);
    assert_eq!(listing.unreadable()[0].instance_id(), &other_instance());
}

/// A prune decides whether a Generation is still in use from the listing, so an entry names the
/// Generation its durable record says it launched from.
#[test]
fn a_listed_sandbox_names_the_generation_it_was_launched_from() {
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::with_state_store(backend, MemoryStateStore::new());
    engine
        .launch_machine(launch_request(operation('1'), instance()))
        .expect("launch");

    let entries = engine.list_machines().expect("listing");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].generation_id().map(GenerationId::as_str),
        Some(format!("sha256:{}", "3".repeat(64)).as_str()),
    );
}

#[test]
fn a_store_that_cannot_be_read_still_fails_the_report() {
    // Only a record that was read and found undecodable is reported. A listing with nothing to
    // stand on must not look like an empty one.
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::with_state_store(backend, UnavailableList);
    assert!(matches!(
        engine.list_machines_reporting_unreadable(),
        Err(ManagedFailure::StateStore(
            StateStoreFailureKind::Unavailable
        ))
    ));
}

struct UnavailableList;

impl StateStore for UnavailableList {
    fn create(
        &mut self,
        _: &InstanceId,
        _: StateRecord,
    ) -> Result<soma::StateRevision, soma::StateStoreFailure> {
        unreachable!("listing does not create")
    }

    fn load(
        &mut self,
        _: &InstanceId,
    ) -> Result<Option<soma::StoredState>, soma::StateStoreFailure> {
        unreachable!("listing never gets as far as a load")
    }

    fn compare_exchange(
        &mut self,
        _: &InstanceId,
        _: soma::StateRevision,
        _: StateRecord,
    ) -> Result<soma::StateRevision, soma::StateStoreFailure> {
        unreachable!("listing does not replace")
    }

    fn list(&mut self) -> Result<Vec<InstanceId>, soma::StateStoreFailure> {
        Err(soma::StateStoreFailure::new(
            StateStoreFailureKind::Unavailable,
        ))
    }
}
