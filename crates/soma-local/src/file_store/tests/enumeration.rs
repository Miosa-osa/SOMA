use std::fs;

use soma::{StateStore, StateStoreFailureKind};

use super::{FileStateStore, TempRoot, instance, record};
use crate::file_store::MACHINE_HOST_DIRECTORY;

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
