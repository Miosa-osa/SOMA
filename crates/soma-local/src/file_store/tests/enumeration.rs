use std::fs;

use soma::StateStore;

use super::{FileStateStore, TempRoot, instance, record};
use crate::file_store::MACHINE_HOST_DIRECTORY;
use crate::prewarm::PERSISTED_FILE;

#[test]
fn a_hosted_root_with_no_records_lists_nothing() {
    let root = TempRoot::new("hosted-empty");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    fs::create_dir_all(root.path().join(MACHINE_HOST_DIRECTORY)).expect("machine directory");

    assert!(
        store
            .list()
            .expect("the machine directory is not a record")
            .is_empty()
    );
}

#[test]
fn records_are_listed_beside_the_machine_directory() {
    let root = TempRoot::new("hosted-records");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    fs::create_dir_all(root.path().join(MACHINE_HOST_DIRECTORY)).expect("machine directory");
    store
        .create(&instance(), record(b"active"))
        .expect("create state");

    assert_eq!(store.list().expect("listed"), vec![instance()]);
}

/// SOMA#228: a stray name is skipped (and reported on the service's log), and the records beside
/// it are still listed. Failing the whole enumeration made every listing on the host fail.
#[test]
fn a_stray_name_is_skipped_and_the_records_beside_it_are_still_listed() {
    let root = TempRoot::new("stray");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    store
        .create(&instance(), record(b"active"))
        .expect("create state");
    fs::create_dir_all(root.path().join("not-an-instance")).expect("stray directory");
    fs::write(root.path().join("autosize.json"), b"{}").expect("stray file");

    assert_eq!(store.list().expect("listed"), vec![instance()]);
}

#[test]
fn the_persisted_prewarm_set_is_not_a_corruption() {
    let root = TempRoot::new("prewarm-file");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    store
        .create(&instance(), record(b"active"))
        .expect("create state");
    fs::write(root.path().join(PERSISTED_FILE), b"{}").expect("the persisted set");
    fs::write(root.path().join(format!("{PERSISTED_FILE}.tmp")), b"{}")
        .expect("its temporary file");

    assert_eq!(store.list().expect("listed"), vec![instance()]);
}

#[test]
fn a_directory_under_the_prewarm_name_is_skipped_and_never_listed() {
    let root = TempRoot::new("prewarm-link");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    fs::create_dir_all(root.path().join(PERSISTED_FILE)).expect("a directory by that name");

    assert!(store.list().expect("listed").is_empty());
}

#[test]
fn the_service_directory_is_not_a_record() {
    let root = TempRoot::new("service");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    fs::create_dir_all(root.path().join(crate::file_store::SERVICE_DIRECTORY)).expect("service");

    assert!(store.list().expect("listed").is_empty());
}
