use std::fs;

use soma::{StateRevision, StateStore, StateStoreFailureKind};

use super::{FileStateStore, INSTANCE, TempRoot, instance, record};
use crate::file_store::layout::{instance_lock_path, revision_path};

#[cfg(unix)]
#[test]
fn symlinked_root_instance_and_revision_paths_are_rejected() {
    use std::os::unix::fs::symlink;

    let base = TempRoot::new("symlinks");
    fs::create_dir_all(base.path()).expect("create test base");
    let real_root = base.path().join("real-root");
    fs::create_dir(&real_root).expect("create real root");
    let linked_root = base.path().join("linked-root");
    symlink(&real_root, &linked_root).expect("link root");
    let root_error = FileStateStore::open(&linked_root).expect_err("linked root is unsafe");
    assert_eq!(root_error.kind(), StateStoreFailureKind::Corrupt);

    let mut store = FileStateStore::open(&real_root).expect("open real root");
    let external_instance = base.path().join("external-instance");
    fs::create_dir(&external_instance).expect("create external instance");
    symlink(&external_instance, real_root.join(INSTANCE)).expect("link instance");
    let instance_error = store
        .load(&instance())
        .expect_err("linked instance is unsafe");
    assert_eq!(instance_error.kind(), StateStoreFailureKind::Corrupt);
    fs::remove_file(real_root.join(INSTANCE)).expect("remove instance link");

    store
        .create(&instance(), record(b"committed"))
        .expect("create state");
    let revision = revision_path(&real_root, &instance(), StateRevision::INITIAL);
    let external_revision = base.path().join("external-revision");
    fs::write(&external_revision, b"replacement").expect("create external revision");
    fs::remove_file(&revision).expect("remove real revision");
    symlink(&external_revision, &revision).expect("link revision");
    let revision_error = store
        .load(&instance())
        .expect_err("linked revision is unsafe");
    assert_eq!(revision_error.kind(), StateStoreFailureKind::Corrupt);
}

#[cfg(unix)]
#[test]
fn multiply_linked_revision_documents_fail_closed() {
    let root = TempRoot::new("hardlinked-revision");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    store
        .create(&instance(), record(b"committed"))
        .expect("create state");
    let revision = revision_path(root.path(), &instance(), StateRevision::INITIAL);
    fs::hard_link(&revision, root.path().join("external-link")).expect("create external hard link");

    let error = store
        .load(&instance())
        .expect_err("multiply linked state is unsafe");

    assert_eq!(error.kind(), StateStoreFailureKind::Corrupt);
}

#[cfg(unix)]
#[test]
fn state_directories_and_documents_are_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = TempRoot::new("permissions");
    let mut store = FileStateStore::open(root.path()).expect("open state store");
    store
        .create(&instance(), record(b"private"))
        .expect("create state");

    let root_mode = fs::metadata(root.path())
        .expect("root metadata")
        .permissions()
        .mode();
    let instance_mode = fs::metadata(root.path().join(INSTANCE))
        .expect("instance metadata")
        .permissions()
        .mode();
    let shard_directory_mode = fs::metadata(root.path().join(".locks"))
        .expect("locks metadata")
        .permissions()
        .mode();
    let shard_file_mode = fs::metadata(instance_lock_path(root.path(), &instance()))
        .expect("lock metadata")
        .permissions()
        .mode();
    let revision_mode = fs::metadata(revision_path(
        root.path(),
        &instance(),
        StateRevision::INITIAL,
    ))
    .expect("revision metadata")
    .permissions()
    .mode();
    assert_eq!(root_mode & 0o777, 0o700);
    assert_eq!(shard_directory_mode & 0o777, 0o700);
    assert_eq!(shard_file_mode & 0o777, 0o600);
    assert_eq!(instance_mode & 0o777, 0o700);
    assert_eq!(revision_mode & 0o777, 0o600);
}
