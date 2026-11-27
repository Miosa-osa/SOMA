use std::{fs, io::Cursor, os::unix::fs::PermissionsExt as _, path::Path};

use super::{CheckpointArtifact, CheckpointFailure, ImportRefusal, ImportStage, limits};

fn private_dir(path: &Path) {
    fs::create_dir(path).expect("stage");
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).expect("private");
}

fn stage() -> (tempfile::TempDir, ImportStage) {
    let root = tempfile::tempdir().expect("root");
    let path = root.path().join(".staging-import-test");
    private_dir(&path);
    let stage = ImportStage::lock(path).expect("locked");
    (root, stage)
}

#[test]
fn an_artifact_is_received_once_with_its_digest() {
    let (_root, stage) = stage();
    let bytes = vec![7_u8; 3 * 1024 * 1024 + 5];
    let digest = stage
        .receive(
            CheckpointArtifact::Memory,
            bytes.len() as u64,
            &mut Cursor::new(&bytes),
        )
        .expect("received");
    assert_eq!(
        digest,
        format!("sha256:{}", soma_kvm::snapshot::Digest::of(&bytes))
    );
    let written = stage.path().join(CheckpointArtifact::Memory.filename());
    assert_eq!(fs::read(&written).expect("written"), bytes);
    assert_eq!(
        fs::metadata(&written).expect("meta").permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        stage.receive(CheckpointArtifact::Memory, 1, &mut Cursor::new(b"x")),
        Err(ImportRefusal::Checkpoint(CheckpointFailure::CaptureRefused)),
    );
}

#[test]
fn a_short_transfer_leaves_nothing_and_can_be_sent_again() {
    let (_root, stage) = stage();
    assert_eq!(
        stage.receive(CheckpointArtifact::State, 10, &mut Cursor::new(b"short")),
        Err(ImportRefusal::Checkpoint(CheckpointFailure::Storage)),
    );
    let target = stage.path().join(CheckpointArtifact::State.filename());
    assert!(!target.exists());
    assert_eq!(fs::read_dir(stage.path()).expect("stage").count(), 0);
    stage
        .receive(CheckpointArtifact::State, 5, &mut Cursor::new(b"whole"))
        .expect("the retry is received");
    assert_eq!(fs::read(target).expect("state"), b"whole");
}

#[test]
fn a_partial_file_a_crash_left_behind_does_not_block_the_resend() {
    let (_root, stage) = stage();
    let part = stage
        .path()
        .join(format!("{}.part", CheckpointArtifact::Overlay.filename()));
    fs::write(&part, b"half of it").expect("left by a crash");
    stage
        .receive(CheckpointArtifact::Overlay, 4, &mut Cursor::new(b"abcdef"))
        .expect("the resend is received");
    assert!(!part.exists());
    assert_eq!(
        fs::read(stage.path().join(CheckpointArtifact::Overlay.filename())).expect("overlay"),
        b"abcd"
    );
}

#[test]
fn one_import_is_held_by_one_request_at_a_time() {
    let (_root, stage) = stage();
    assert_eq!(
        ImportStage::lock(stage.path().to_path_buf()).map(|_| ()),
        Err(ImportRefusal::Busy)
    );
    let path = stage.path().to_path_buf();
    drop(stage);
    ImportStage::lock(path).expect("free once the holder answered");
}

#[test]
fn an_abort_removes_the_stage_and_what_it_received() {
    let (_root, stage) = stage();
    stage
        .receive(CheckpointArtifact::Workspace, 3, &mut Cursor::new(b"vol"))
        .expect("received");
    let path = stage.path().to_path_buf();
    stage.abort().expect("aborted");
    assert!(!path.exists());
}

#[test]
fn an_abandoned_stage_expires_and_a_held_one_does_not() {
    let root = tempfile::tempdir().expect("root");
    let old = root.path().join(".staging-import-old");
    let held = root.path().join(".staging-import-held");
    private_dir(&old);
    private_dir(&held);
    fs::write(old.join("memory.raw"), b"x").expect("received");
    let long_ago = rustix::fs::Timestamps {
        last_access: rustix::fs::Timespec {
            tv_sec: 1,
            tv_nsec: 0,
        },
        last_modification: rustix::fs::Timespec {
            tv_sec: 1,
            tv_nsec: 0,
        },
    };
    for stage in [&old, &held] {
        rustix::fs::utimensat(
            rustix::fs::CWD,
            stage,
            &long_ago,
            rustix::fs::AtFlags::empty(),
        )
        .expect("aged");
    }
    let holder = ImportStage::lock(held.clone()).expect("held");
    assert_eq!(limits::open_imports(root.path()), 2);
    limits::sweep_expired(root.path());
    assert!(!old.exists(), "an abandoned stage is removed");
    assert!(held.exists(), "a stage a request holds is kept");
    drop(holder);
}

#[test]
fn an_artifact_that_does_not_fit_is_refused_before_a_byte_is_read() {
    let (root, stage) = stage();
    let mut source = Cursor::new(vec![0_u8; 8]);
    assert_eq!(
        stage.receive(
            CheckpointArtifact::Memory,
            limits::MAX_STAGED_BYTES + 1,
            &mut source
        ),
        Err(ImportRefusal::NoSpace)
    );
    assert_eq!(source.position(), 0);
    assert!(limits::require_room(root.path(), 1).is_ok());
}
