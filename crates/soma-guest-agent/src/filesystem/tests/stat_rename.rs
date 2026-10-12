//! Stat, rename, and the guest API probe, against a real temporary directory.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use soma_guest::{EntryKind, FileFailure, FileOutcome, FileRequest};
use tempfile::TempDir;

use super::{at, perform, read};
use crate::filesystem::{GUEST_API_LEVEL, GUEST_API_PROBE_PREFIX};

#[test]
fn stat_reports_kind_size_and_mode() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("note");
    fs::write(&path, b"hello").expect("seed file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).expect("mode");

    let FileOutcome::Stat {
        kind,
        size,
        mode,
        modified,
    } = perform(&FileRequest::Stat { path: at(&path) })
    else {
        panic!("a stat answer");
    };
    assert_eq!((kind, size, mode), (EntryKind::File, 5, 0o640));
    assert!(modified > 1_000_000_000);

    let FileOutcome::Stat { kind, size, .. } = perform(&FileRequest::Stat {
        path: at(root.path()),
    }) else {
        panic!("a stat answer");
    };
    assert_eq!((kind, size), (EntryKind::Directory, 0));
}

#[test]
fn stat_of_a_missing_path_is_not_found_and_a_link_is_not_followed() {
    let root = TempDir::new().expect("temporary directory");
    assert_eq!(
        perform(&FileRequest::Stat {
            path: at(&root.path().join("absent"))
        }),
        FileOutcome::Failed(FileFailure::NotFound)
    );
    let link = root.path().join("link");
    std::os::unix::fs::symlink("/nonexistent", &link).expect("dangling link");
    let FileOutcome::Stat { kind, .. } = perform(&FileRequest::Stat { path: at(&link) }) else {
        panic!("a link is itself a path");
    };
    assert_eq!(kind, EntryKind::Other);
}

#[test]
fn rename_replaces_a_file_atomically_and_keeps_its_mode() {
    let root = TempDir::new().expect("temporary directory");
    let from = root.path().join(".tmp");
    let to = root.path().join("final");
    fs::write(&to, b"old").expect("seed target");
    fs::write(&from, b"new").expect("seed source");
    fs::set_permissions(&from, fs::Permissions::from_mode(0o600)).expect("mode");

    let outcome = perform(&FileRequest::Rename {
        path: at(&from),
        to: at(&to),
    });
    assert_eq!(outcome, FileOutcome::Done);
    assert_eq!(fs::read(&to).expect("target"), b"new");
    assert!(!from.exists());
    assert_eq!(
        fs::metadata(&to).expect("meta").permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn rename_reports_missing_source_and_kind_mismatch() {
    let root = TempDir::new().expect("temporary directory");
    let file = root.path().join("file");
    let dir = root.path().join("dir");
    fs::write(&file, b"x").expect("seed");
    fs::create_dir(&dir).expect("dir");

    assert_eq!(
        perform(&FileRequest::Rename {
            path: at(&root.path().join("absent")),
            to: at(&file),
        }),
        FileOutcome::Failed(FileFailure::NotFound)
    );
    assert_eq!(
        perform(&FileRequest::Rename {
            path: at(&file),
            to: at(&dir),
        }),
        FileOutcome::Failed(FileFailure::WrongKind)
    );
}

fn probe_path(nonce: &str) -> std::path::PathBuf {
    let prefix = std::str::from_utf8(GUEST_API_PROBE_PREFIX).expect("text");
    std::path::PathBuf::from(format!("{prefix}{nonce}"))
}

#[test]
fn the_probe_echoes_the_nonce_with_the_level_and_honours_the_offset() {
    assert_eq!(
        read(&probe_path("abc123"), 0, 64),
        FileOutcome::Read {
            bytes: format!("{GUEST_API_LEVEL} abc123\n").into_bytes().into(),
            end: true
        }
    );
    assert_eq!(
        read(&probe_path("abc123"), 99, 64),
        FileOutcome::Read {
            bytes: Box::default(),
            end: true
        }
    );
}

#[test]
fn a_probe_without_a_usable_nonce_is_refused_and_never_reads_the_disk() {
    for nonce in ["", "a/b", "a b", &"x".repeat(65)] {
        assert_eq!(
            read(&probe_path(nonce), 0, 64),
            FileOutcome::Failed(FileFailure::NotFound),
            "{nonce:?}"
        );
    }
}
