use std::fs;

use soma::StateStore;

use super::{FileStateStore, TempRoot};

#[test]
fn hosted_machine_directory_is_not_enumerated_as_an_instance() {
    let root = TempRoot::new("hosted-machine-directory");
    let mut store = FileStateStore::open(root.path()).expect("open state store");

    fs::create_dir(root.path().join("machines")).expect("create hosted machine directory");

    assert_eq!(store.list().expect("list instances"), Vec::new());
}
