//! The saved automatic sizing never sits among the state store's records (SOMA#227).

use std::{
    fs,
    path::{Path, PathBuf},
};

use soma::{StateRecord, StateStore as _};

/// A directory of its own for one test, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("soma-autosize-root-{name}-{}", std::process::id()));
        let _ignored = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("scratch directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.0);
    }
}

fn instance() -> soma::InstanceId {
    soma::InstanceId::new("0123456789abcdef0123456789abcdef").expect("instance")
}

#[test]
fn a_saved_sizing_where_the_service_keeps_it_leaves_the_records_listable() {
    let root = Scratch::new("listable");
    let mut store = soma_local::FileStateStore::open(root.path()).expect("open state store");
    store
        .create(
            &instance(),
            StateRecord::from_bytes(b"active".to_vec()).expect("record"),
        )
        .expect("create state");
    // Where soma-api's startup keeps the saved sizing: the state root joined with this name.
    let saved = root.path().join(soma_api::autosize::SAVED_FILE);
    fs::create_dir_all(saved.parent().expect("a parent")).expect("parent");
    fs::write(&saved, br#"{"sized":{}}"#).expect("saved sizing");

    assert_eq!(
        store
            .list()
            .expect("the saved sizing is not a stray record"),
        vec![instance()]
    );
}

#[test]
fn the_saved_sizing_lives_in_the_directory_the_store_skips() {
    assert_eq!(
        Path::new(soma_api::autosize::SAVED_FILE).parent(),
        Some(Path::new(soma_local::SERVICE_DIRECTORY))
    );
}

#[test]
fn a_sizing_an_earlier_build_left_among_the_records_is_moved_once_and_kept() {
    let root = Scratch::new("legacy");
    let mut store = soma_local::FileStateStore::open(root.path()).expect("open state store");
    let legacy = root.path().join("autosize.json");
    let sized = br#"{"sized":{"sbx-a":{"min_cpu":1,"max_cpu":4,"min_memory_mb":1024,"max_memory_mb":4096}}}"#;
    fs::write(&legacy, sized).expect("legacy file");

    soma_api::autosize::adopt_legacy(root.path());

    assert!(!legacy.exists());
    let current = root.path().join(soma_api::autosize::SAVED_FILE);
    assert_eq!(fs::read(&current).expect("moved"), sized);
    assert!(store.list().expect("listable again").is_empty());

    // A newer file already in place is kept, and a stale legacy copy is only removed.
    fs::write(&legacy, br#"{"sized":{}}"#).expect("stale legacy file");
    soma_api::autosize::adopt_legacy(root.path());
    assert!(!legacy.exists());
    assert_eq!(fs::read(&current).expect("kept"), sized);
}
