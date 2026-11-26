use std::{fs, io::Cursor, os::unix::fs::PermissionsExt as _};

use super::{CheckpointArtifact, CheckpointFailure, ImportStage};

fn stage() -> (tempfile::TempDir, ImportStage) {
    let root = tempfile::tempdir().expect("root");
    let path = root.path().join(".staging-import-test");
    fs::create_dir(&path).expect("stage");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("private");
    (root, ImportStage { path })
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
        Err(CheckpointFailure::CaptureRefused),
    );
}

#[test]
fn a_short_transfer_leaves_nothing_and_can_be_sent_again() {
    let (_root, stage) = stage();
    assert_eq!(
        stage.receive(CheckpointArtifact::State, 10, &mut Cursor::new(b"short")),
        Err(CheckpointFailure::Storage),
    );
    let target = stage.path().join(CheckpointArtifact::State.filename());
    assert!(!target.exists());
    stage
        .receive(CheckpointArtifact::State, 5, &mut Cursor::new(b"whole"))
        .expect("the retry is received");
    assert_eq!(fs::read(target).expect("state"), b"whole");
}

#[test]
fn a_source_longer_than_its_length_is_read_only_to_the_length() {
    let (_root, stage) = stage();
    let mut source = Cursor::new(b"abcdef".to_vec());
    stage
        .receive(CheckpointArtifact::Overlay, 4, &mut source)
        .expect("received");
    assert_eq!(source.position(), 4);
    assert_eq!(
        fs::read(stage.path().join(CheckpointArtifact::Overlay.filename())).expect("overlay"),
        b"abcd"
    );
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
