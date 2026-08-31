//! Creating a file at a chosen mode, and changing the mode of one that exists.
//!
//! The assertions are about the mode the kernel reports, not about the mode the request asked
//! for, because the whole reason this operation exists is that an ambient umask would otherwise
//! decide what a delivered credential is readable by.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use soma_guest::{FileFailure, FileOutcome, FileRequest};
use tempfile::TempDir;

use super::{at, perform};

/// The permission bits of one path, without the file-type bits the kernel packs beside them.
fn mode_of(path: &std::path::Path) -> u32 {
    fs::symlink_metadata(path)
        .expect("the path exists")
        .permissions()
        .mode()
        & 0o7777
}

#[test]
fn a_created_file_has_exactly_the_mode_that_was_asked_for() {
    let directory = TempDir::new().expect("temporary directory");
    let path = directory.path().join("token");

    let outcome = perform(&FileRequest::Create {
        path: at(&path),
        mode: 0o600,
    });

    assert_eq!(outcome, FileOutcome::Done);
    assert_eq!(mode_of(&path), 0o600);
    assert_eq!(fs::read(&path).expect("an empty new file"), Vec::new());
}

#[test]
fn a_created_file_is_not_widened_or_narrowed_by_the_ambient_mask() {
    let directory = TempDir::new().expect("temporary directory");
    let path = directory.path().join("token");

    // A default mask clears the group and other write bits, so a mode carrying them proves the
    // creation set the mode itself rather than letting the mask choose it.
    let outcome = perform(&FileRequest::Create {
        path: at(&path),
        mode: 0o666,
    });

    assert_eq!(outcome, FileOutcome::Done);
    assert_eq!(mode_of(&path), 0o666);
}

#[test]
fn creating_a_path_that_already_exists_is_refused() {
    let directory = TempDir::new().expect("temporary directory");
    let path = directory.path().join("token");
    fs::write(&path, b"someone else's file").expect("an existing file");

    let outcome = perform(&FileRequest::Create {
        path: at(&path),
        mode: 0o600,
    });

    assert_eq!(outcome, FileOutcome::Failed(FileFailure::Exists));
    assert_eq!(
        fs::read(&path).expect("the existing file"),
        b"someone else's file"
    );
}

#[test]
fn setting_the_mode_of_an_existing_file_reports_the_new_mode() {
    let directory = TempDir::new().expect("temporary directory");
    let path = directory.path().join("token");
    fs::write(&path, b"value").expect("a written file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("an initial mode");

    let outcome = perform(&FileRequest::SetMode {
        path: at(&path),
        mode: 0o400,
    });

    assert_eq!(outcome, FileOutcome::Done);
    assert_eq!(mode_of(&path), 0o400);
}

#[test]
fn setting_the_mode_of_a_path_that_does_not_exist_is_refused() {
    let directory = TempDir::new().expect("temporary directory");

    let outcome = perform(&FileRequest::SetMode {
        path: at(&directory.path().join("absent")),
        mode: 0o400,
    });

    assert_eq!(outcome, FileOutcome::Failed(FileFailure::NotFound));
}
