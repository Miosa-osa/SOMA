use std::fs;

use soma::{StateStore, StateStoreFailureKind};

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

#[test]
fn any_other_stray_name_is_still_a_corruption() {
    let root = TempRoot::new("stray");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    fs::create_dir_all(root.path().join("not-an-instance")).expect("stray directory");

    assert_eq!(
        store.list().expect_err("a stray name").kind(),
        StateStoreFailureKind::Corrupt
    );
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
fn a_link_or_directory_under_the_prewarm_name_is_still_a_corruption() {
    let root = TempRoot::new("prewarm-link");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    fs::create_dir_all(root.path().join(PERSISTED_FILE)).expect("a directory by that name");

    assert_eq!(
        store
            .list()
            .expect_err("a directory is not the file")
            .kind(),
        StateStoreFailureKind::Corrupt
    );
}
