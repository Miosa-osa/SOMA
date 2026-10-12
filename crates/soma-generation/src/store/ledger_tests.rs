use std::sync::atomic::Ordering;

use crate::{
    ImportErrorKind, ImportPhase,
    oci::{Descriptor, PLAIN_LAYER},
};

fn store_with(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf, Descriptor) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("store");
    std::fs::create_dir(&root).unwrap();
    let store = super::Store::open(&root).unwrap();
    let descriptor = store
        .put_bytes(bytes, PLAIN_LAYER, ImportPhase::Publish)
        .unwrap();
    (temporary, root, descriptor)
}

fn admit(store: &super::Store, descriptor: &Descriptor) -> Result<(), ImportErrorKind> {
    store
        .open_admitted_blob(descriptor, descriptor.size, ImportPhase::Publish)
        .map(|_| ())
        .map_err(|error| error.kind())
}

fn blob(root: &std::path::Path, descriptor: &Descriptor) -> std::path::PathBuf {
    root.join("v1/blobs/sha256")
        .join(crate::digest::hex(&descriptor.digest))
}

#[test]
fn an_unchanged_blob_is_hashed_once_and_then_admitted_without_a_read() {
    let (_keep, root, descriptor) = store_with(b"a launch artifact");
    let store = super::Store::open(&root).unwrap();
    admit(&store, &descriptor).unwrap();
    admit(&store, &descriptor).unwrap();
    assert_eq!(store.hashed.load(Ordering::Relaxed), 1);
}

#[test]
fn the_record_survives_a_restart() {
    let (_keep, root, descriptor) = store_with(b"a launch artifact");
    admit(&super::Store::open(&root).unwrap(), &descriptor).unwrap();
    let restarted = super::Store::open(&root).unwrap();
    admit(&restarted, &descriptor).unwrap();
    assert_eq!(restarted.hashed.load(Ordering::Relaxed), 0);
}

#[test]
fn a_blob_changed_after_it_was_recorded_is_hashed_again_and_refused() {
    let (_keep, root, descriptor) = store_with(b"a launch artifact");
    admit(&super::Store::open(&root).unwrap(), &descriptor).unwrap();
    let path = blob(&root, &descriptor);
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(25));
    std::fs::write(&path, b"a launch artifacX").unwrap();
    let restarted = super::Store::open(&root).unwrap();
    assert_eq!(
        admit(&restarted, &descriptor),
        Err(ImportErrorKind::StoreConflict)
    );
    assert_eq!(restarted.hashed.load(Ordering::Relaxed), 1);
}

#[test]
fn a_damaged_ledger_means_every_blob_is_hashed() {
    let (_keep, root, descriptor) = store_with(b"a launch artifact");
    admit(&super::Store::open(&root).unwrap(), &descriptor).unwrap();
    std::fs::write(root.join("v1/admitted-ledger"), "garbage\n1 2 3\n").unwrap();
    let restarted = super::Store::open(&root).unwrap();
    admit(&restarted, &descriptor).unwrap();
    assert_eq!(restarted.hashed.load(Ordering::Relaxed), 1);
}

#[test]
fn a_ledger_written_for_another_inode_does_not_vouch_for_this_blob() {
    let (_keep, root, descriptor) = store_with(b"a launch artifact");
    admit(&super::Store::open(&root).unwrap(), &descriptor).unwrap();
    let ledger = std::fs::read_to_string(root.join("v1/admitted-ledger")).unwrap();
    let mut fields: Vec<String> = ledger.trim().split(' ').map(str::to_owned).collect();
    fields[2] = "1".to_owned();
    std::fs::write(root.join("v1/admitted-ledger"), fields.join(" ")).unwrap();
    let restarted = super::Store::open(&root).unwrap();
    admit(&restarted, &descriptor).unwrap();
    assert_eq!(restarted.hashed.load(Ordering::Relaxed), 1);
}
