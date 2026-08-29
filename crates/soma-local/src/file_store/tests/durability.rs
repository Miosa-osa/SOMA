use std::fs;

use soma::{StateRevision, StateStore, StateStoreFailureKind};

use super::{FileStateStore, INSTANCE, TempRoot, instance, record};
use crate::file_store::layout::revision_path;

#[test]
fn state_survives_store_restart_byte_for_byte() {
    let root = TempRoot::new("restart");
    let mut first = FileStateStore::open(root.path()).expect("open state store");

    let revision = first
        .create(&instance(), record(br#"{"state":"active"}"#))
        .expect("create state");
    drop(first);

    let mut restarted = FileStateStore::open(root.path()).expect("reopen state store");
    let stored = restarted
        .load(&instance())
        .expect("load state")
        .expect("state exists");
    assert_eq!(revision, StateRevision::INITIAL);
    assert_eq!(stored.revision(), StateRevision::INITIAL);
    assert_eq!(stored.record().as_bytes(), br#"{"state":"active"}"#);
}

#[test]
fn absent_load_and_compare_exchange_do_not_create_instance_directories() {
    let root = TempRoot::new("absent-read");
    let mut store = FileStateStore::open(root.path()).expect("open state store");

    assert!(
        store
            .load(&instance())
            .expect("load absent state")
            .is_none()
    );
    assert!(!root.path().join(INSTANCE).exists());

    let error = store
        .compare_exchange(
            &instance(),
            StateRevision::INITIAL,
            record(b"must not be written"),
        )
        .expect_err("absent state conflicts");

    assert_eq!(error.kind(), StateStoreFailureKind::Conflict);
    assert!(!root.path().join(INSTANCE).exists());
}

#[test]
fn compare_exchange_is_atomic_and_prunes_superseded_revisions() {
    let root = TempRoot::new("compare-exchange");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    store
        .create(&instance(), record(b"revision one"))
        .expect("create state");

    let revision = store
        .compare_exchange(&instance(), StateRevision::INITIAL, record(b"revision two"))
        .expect("replace state");

    assert_eq!(revision.get(), 2);
    assert!(!revision_path(root.path(), &instance(), StateRevision::INITIAL).exists());
    assert!(revision_path(root.path(), &instance(), revision).exists());
    let conflict = store
        .compare_exchange(
            &instance(),
            StateRevision::INITIAL,
            record(b"stale replacement"),
        )
        .expect_err("stale revision conflicts");
    assert_eq!(conflict.kind(), StateStoreFailureKind::Conflict);
}

#[test]
fn revision_overflow_is_a_capacity_failure_without_mutation() {
    let root = TempRoot::new("revision-overflow");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    store
        .create(&instance(), record(b"last possible revision"))
        .expect("create state");
    let maximum = StateRevision::new(u64::MAX).expect("maximum revision");
    fs::rename(
        revision_path(root.path(), &instance(), StateRevision::INITIAL),
        revision_path(root.path(), &instance(), maximum),
    )
    .expect("inject maximum revision");

    let error = store
        .compare_exchange(&instance(), maximum, record(b"overflow"))
        .expect_err("revision cannot wrap");

    assert_eq!(error.kind(), StateStoreFailureKind::CapacityExceeded);
    let stored = store
        .load(&instance())
        .expect("load state")
        .expect("state exists");
    assert_eq!(stored.revision(), maximum);
    assert_eq!(stored.record().as_bytes(), b"last possible revision");
}
