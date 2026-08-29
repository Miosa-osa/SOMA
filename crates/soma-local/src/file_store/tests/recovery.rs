use std::{
    fs::{self, File},
    io::Write as _,
};

use soma::{MAX_STATE_RECORD_BYTES, StateRevision, StateStore, StateStoreFailureKind};

use super::{FileStateStore, INSTANCE, TempRoot, instance, record};
use crate::file_store::layout::revision_path;

#[test]
fn corrupt_highest_revision_fails_closed_without_falling_back() {
    let root = TempRoot::new("corruption");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    store
        .create(&instance(), record(b"known-good"))
        .expect("create state");
    let corrupt_revision = StateRevision::new(2).expect("revision two");
    fs::write(
        revision_path(root.path(), &instance(), corrupt_revision),
        Vec::<u8>::new(),
    )
    .expect("inject corrupt highest revision");

    let error = store
        .load(&instance())
        .expect_err("highest corrupt revision must not fall back");

    assert_eq!(error.kind(), StateStoreFailureKind::Corrupt);
}

#[test]
fn compare_exchange_never_overwrites_corrupt_current_state() {
    let root = TempRoot::new("corrupt-cas");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    store
        .create(&instance(), record(b"known-good"))
        .expect("create state");
    fs::write(
        revision_path(root.path(), &instance(), StateRevision::INITIAL),
        Vec::<u8>::new(),
    )
    .expect("corrupt current revision");

    let error = store
        .compare_exchange(
            &instance(),
            StateRevision::INITIAL,
            record(b"must not replace corruption"),
        )
        .expect_err("corruption cannot be advanced past");

    assert_eq!(error.kind(), StateStoreFailureKind::Corrupt);
    assert!(
        !revision_path(
            root.path(),
            &instance(),
            StateRevision::new(2).expect("revision two")
        )
        .exists()
    );
}

#[test]
fn oversized_highest_revision_is_corrupt_without_unbounded_read() {
    let root = TempRoot::new("oversized");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    store
        .create(&instance(), record(b"known-good"))
        .expect("create state");
    let oversized = StateRevision::new(2).expect("revision two");
    let mut file = File::create(revision_path(root.path(), &instance(), oversized))
        .expect("create oversized revision");
    file.write_all(&vec![b'x'; MAX_STATE_RECORD_BYTES + 1])
        .expect("write oversized revision");

    let error = store
        .load(&instance())
        .expect_err("oversized highest revision is corrupt");

    assert_eq!(error.kind(), StateStoreFailureKind::Corrupt);
}

#[test]
fn a_valid_interrupted_temp_write_is_removed_before_loading_committed_state() {
    let root = TempRoot::new("interrupted-temp");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    store
        .create(&instance(), record(b"committed"))
        .expect("create state");
    let temp = root
        .path()
        .join(INSTANCE)
        .join(".tmp-0000000001-00000000000000000001");
    fs::write(&temp, b"partial").expect("inject interrupted write");

    let stored = store
        .load(&instance())
        .expect("load state")
        .expect("state exists");

    assert_eq!(stored.record().as_bytes(), b"committed");
    assert!(!temp.exists());
}

#[cfg(unix)]
#[test]
fn publication_recovers_when_crash_leaves_the_committed_temp_link() {
    let root = TempRoot::new("interrupted-publication");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    store
        .create(&instance(), record(b"committed"))
        .expect("create state");
    let revision = revision_path(root.path(), &instance(), StateRevision::INITIAL);
    let temp = root
        .path()
        .join(INSTANCE)
        .join(".tmp-0000000001-00000000000000000001");
    fs::hard_link(&revision, &temp).expect("recreate committed temp link");

    let stored = store
        .load(&instance())
        .expect("recover state")
        .expect("state exists");

    assert_eq!(stored.record().as_bytes(), b"committed");
    assert!(!temp.exists());
}

#[test]
fn malformed_temp_and_noncanonical_revision_names_fail_closed() {
    for name in [".tmp-unbounded", "1.state"] {
        let root = TempRoot::new("ambiguous-name");
        let mut store = FileStateStore::open(root.path()).expect("open state store");
        store
            .create(&instance(), record(b"committed"))
            .expect("create state");
        fs::write(root.path().join(INSTANCE).join(name), b"ambiguous")
            .expect("inject ambiguous entry");

        let error = store
            .load(&instance())
            .expect_err("ambiguous entry fails closed");

        assert_eq!(error.kind(), StateStoreFailureKind::Corrupt);
    }
}
