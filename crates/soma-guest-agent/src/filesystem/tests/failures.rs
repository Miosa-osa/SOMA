//! Every cause the protocol admits, produced by a real filesystem rather than a stub, and the
//! refusals that keep a request the protocol could not carry from reaching the kernel at all.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use soma_guest::{FileFailure, FileOutcome, FileRequest};
use tempfile::TempDir;

use super::{at, perform, read, write};

/// Builds a request whose path never came off the wire.
fn exists(path: &[u8]) -> FileOutcome {
    perform(&FileRequest::Exists { path: path.into() })
}

#[test]
fn a_missing_file_is_not_found() {
    let root = TempDir::new().expect("temporary directory");

    assert_eq!(
        read(&root.path().join("gone"), 0, 8),
        FileOutcome::Failed(FileFailure::NotFound)
    );
}

#[test]
fn writing_without_asking_to_create_is_not_found() {
    let root = TempDir::new().expect("temporary directory");

    assert_eq!(
        write(&root.path().join("gone"), 0, false, false, b"x"),
        FileOutcome::Failed(FileFailure::NotFound)
    );
}

#[test]
fn removing_a_missing_path_is_not_found() {
    let root = TempDir::new().expect("temporary directory");

    assert_eq!(
        perform(&FileRequest::Remove {
            path: at(&root.path().join("gone")),
            recursive: true,
        }),
        FileOutcome::Failed(FileFailure::NotFound)
    );
}

#[test]
fn an_unreadable_file_is_denied() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("secret");
    fs::write(&path, b"x").expect("seed file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("remove every mode bit");

    let outcome = read(&path, 0, 8);

    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("restore mode bits");
    // A privileged process is refused nothing by the mode bits, so this case cannot be produced
    // as root and the outcome then says nothing about the mapping under test.
    if matches!(outcome, FileOutcome::Read { .. }) {
        return;
    }
    assert_eq!(outcome, FileOutcome::Failed(FileFailure::Denied));
}

#[test]
fn reading_a_directory_is_the_wrong_kind() {
    let root = TempDir::new().expect("temporary directory");

    assert_eq!(
        read(root.path(), 0, 8),
        FileOutcome::Failed(FileFailure::WrongKind)
    );
}

#[test]
fn writing_to_a_directory_is_the_wrong_kind() {
    let root = TempDir::new().expect("temporary directory");

    assert_eq!(
        write(root.path(), 0, false, false, b"x"),
        FileOutcome::Failed(FileFailure::WrongKind)
    );
}

#[test]
fn listing_a_file_is_the_wrong_kind() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("note");
    fs::write(&path, b"x").expect("seed file");

    assert_eq!(
        perform(&FileRequest::ReadDirectory {
            path: at(&path),
            offset: 0,
        }),
        FileOutcome::Failed(FileFailure::WrongKind)
    );
}

#[test]
fn making_a_directory_that_is_already_there_already_exists() {
    let root = TempDir::new().expect("temporary directory");

    assert_eq!(
        perform(&FileRequest::MakeDirectory {
            path: at(root.path()),
            parents: false,
        }),
        FileOutcome::Failed(FileFailure::Exists)
    );
}

#[test]
fn removing_a_populated_directory_without_recursing_is_not_empty() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("tree");
    fs::create_dir(&path).expect("seed directory");
    fs::write(path.join("note"), b"x").expect("seed file");

    assert_eq!(
        perform(&FileRequest::Remove {
            path: at(&path),
            recursive: false,
        }),
        FileOutcome::Failed(FileFailure::NotEmpty)
    );
}

#[test]
fn a_path_carrying_an_interior_nul_is_refused() {
    // The kernel would read such a path only as far as the nul, so the guest would answer about
    // a file the request never named.
    assert_eq!(
        exists(b"/tmp\0/elsewhere"),
        FileOutcome::Failed(FileFailure::Failed)
    );
}

#[test]
fn a_relative_path_is_refused() {
    assert_eq!(
        exists(b"relative/note"),
        FileOutcome::Failed(FileFailure::Failed)
    );
}

#[test]
fn an_empty_path_is_refused() {
    assert_eq!(exists(b""), FileOutcome::Failed(FileFailure::Failed));
}

#[test]
fn a_path_longer_than_the_protocol_carries_is_refused() {
    let mut path = vec![b'a'; soma_guest::MAX_PATH_BYTES + 1];
    path[0] = b'/';

    assert_eq!(exists(&path), FileOutcome::Failed(FileFailure::Failed));
}
