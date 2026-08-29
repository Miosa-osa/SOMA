use soma::{
    InstanceId, MAX_STATE_RECORD_BYTES, MemoryStateStore, StateRecord, StateRevision, StateStore,
    StateStoreFailureKind,
};

#[test]
fn memory_store_is_process_local_but_shared_by_its_clones() {
    let key = instance();
    let mut first = MemoryStateStore::new();
    let mut restarted = first.clone();
    let initial = StateRecord::from_bytes(b"initial".to_vec()).expect("bounded record");

    let revision = first.create(&key, initial.clone()).expect("create");
    let loaded = restarted.load(&key).expect("load").expect("record exists");

    assert_eq!(revision, StateRevision::INITIAL);
    assert_eq!(loaded.revision(), revision);
    assert_eq!(loaded.record(), &initial);
}

#[test]
fn create_and_compare_exchange_conflicts_fail_closed() {
    let key = instance();
    let mut store = MemoryStateStore::new();
    let initial = StateRecord::from_bytes(b"initial".to_vec()).expect("record");
    let replacement = StateRecord::from_bytes(b"replacement".to_vec()).expect("record");
    store.create(&key, initial).expect("create");

    let create_conflict = store
        .create(&key, replacement.clone())
        .expect_err("create must be exclusive");
    let revision_conflict = store
        .compare_exchange(
            &key,
            StateRevision::new(2).expect("revision"),
            replacement.clone(),
        )
        .expect_err("stale revision must fail");

    assert_eq!(create_conflict.kind(), StateStoreFailureKind::Conflict);
    assert_eq!(revision_conflict.kind(), StateStoreFailureKind::Conflict);
    let next = store
        .compare_exchange(&key, StateRevision::INITIAL, replacement.clone())
        .expect("current revision updates");
    assert_eq!(next, StateRevision::new(2).expect("revision"));
    assert_eq!(
        store.load(&key).expect("load").expect("record").record(),
        &replacement
    );
}

#[test]
fn records_and_revisions_enforce_portable_bounds() {
    assert!(StateRecord::from_bytes(Vec::new()).is_err());
    assert!(StateRecord::from_bytes(vec![0; MAX_STATE_RECORD_BYTES]).is_ok());
    assert!(StateRecord::from_bytes(vec![0; MAX_STATE_RECORD_BYTES + 1]).is_err());
    assert!(StateRevision::new(0).is_err());
}

fn instance() -> InstanceId {
    InstanceId::new("22222222222222222222222222222222").expect("instance")
}
