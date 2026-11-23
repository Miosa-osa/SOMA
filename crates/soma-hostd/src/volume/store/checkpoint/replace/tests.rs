use super::*;
use crate::{RequestFingerprint, VolumeId, volume::VolumeStore};
use std::{
    fs::OpenOptions,
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _},
};

const TRANSITION: u64 = 11;

fn private(path: &Path, contents: &[u8]) -> File {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(contents).unwrap();
    file.sync_all().unwrap();
    file
}

fn stage(root: &Path) -> (PathBuf, PathBuf, Intent, File) {
    let backing = root.join(VOLUME_FILE);
    let path = root.join(".restore-test");
    fs::create_dir(&path).unwrap();
    let original = private(&backing, b"post-checkpoint writes");
    let replacement = private(&path.join(VOLUME_FILE), b"checkpoint bytes");
    let intent = Intent {
        fingerprint: [7; 32],
        original: inode(&original).unwrap(),
        replacement: inode(&replacement).unwrap(),
        transition: TRANSITION,
    };
    private(&path.join(INTENT), &intent.encode());
    (backing, path, intent, replacement)
}

#[test]
fn stage_requires_matching_intent_and_publication_requires_the_worker_lock() {
    let root = tempfile::tempdir().unwrap();
    let (backing, path, intent, replacement) = stage(root.path());
    assert!(matches!(
        Intent::read(&path, [8; 32]),
        Err(VolumeError::ChangedIntent)
    ));
    let publish = |transition| {
        publish(
            root.path(),
            &path,
            &backing,
            intent,
            VolumeState::Detached,
            transition,
        )
    };
    assert!(matches!(publish(TRANSITION), Err(VolumeError::Busy(_))));
    lock(&replacement).unwrap();
    assert_eq!(publish(TRANSITION + 1), Err(VolumeError::ChangedIntent));
    assert_eq!(fs::read(backing).unwrap(), b"post-checkpoint writes");
    assert!(!path.join(RETAINED).exists());
}

#[test]
fn publication_preserves_newer_writes_and_can_resume_after_persisting_the_old_link() {
    let root = tempfile::tempdir().unwrap();
    let (backing, path, intent, replacement) = stage(root.path());
    lock(&replacement).unwrap();
    let publish = || {
        publish(
            root.path(),
            &path,
            &backing,
            intent,
            VolumeState::Detached,
            TRANSITION,
        )
    };
    fs::hard_link(&backing, path.join(RETAINED)).unwrap();
    assert_eq!(fs::read(&backing).unwrap(), b"post-checkpoint writes");

    publish().unwrap();
    publish().unwrap();
    assert_eq!(fs::read(&backing).unwrap(), b"checkpoint bytes");
    assert_eq!(
        fs::read(path.join(RETAINED)).unwrap(),
        b"post-checkpoint writes"
    );
    assert!(!path.join(VOLUME_FILE).exists());
    assert_eq!(
        inode(&private_file(&backing, false).unwrap()).unwrap(),
        inode(&replacement).unwrap()
    );
}

#[test]
fn an_unrelated_retained_file_cannot_be_used_as_a_rollback_version() {
    let root = tempfile::tempdir().unwrap();
    let (backing, path, intent, replacement) = stage(root.path());
    lock(&replacement).unwrap();
    private(&path.join(RETAINED), b"other");
    assert_eq!(
        publish(
            root.path(),
            &path,
            &backing,
            intent,
            VolumeState::Detached,
            TRANSITION
        ),
        Err(VolumeError::ChangedIntent)
    );
    assert_eq!(fs::read(backing).unwrap(), b"post-checkpoint writes");
    assert_eq!(
        fs::read(path.join(VOLUME_FILE)).unwrap(),
        b"checkpoint bytes"
    );
}

#[test]
fn rolling_back_requires_a_stopped_worker_and_keeps_its_later_writes() {
    let root = tempfile::tempdir().unwrap();
    let (backing, path, intent, mut replacement) = stage(root.path());
    lock(&replacement).unwrap();
    publish(
        root.path(),
        &path,
        &backing,
        intent,
        VolumeState::Detached,
        TRANSITION,
    )
    .unwrap();

    assert!(matches!(
        rollback(root.path(), &path, &backing, intent),
        Err(VolumeError::Busy(_))
    ));
    replacement.write_all(b" and recovered writes").unwrap();
    replacement.sync_all().unwrap();
    drop(replacement);

    fs::hard_link(&backing, path.join(DISPLACED)).unwrap();
    fs::hard_link(path.join(RETAINED), path.join(ROLLBACK)).unwrap();
    rollback(root.path(), &path, &backing, intent).unwrap();
    rollback(root.path(), &path, &backing, intent).unwrap();

    assert_eq!(fs::read(backing).unwrap(), b"post-checkpoint writes");
    assert_eq!(
        fs::read(path.join(DISPLACED)).unwrap(),
        b"checkpoint bytes and recovered writes"
    );
}

mod restart;
