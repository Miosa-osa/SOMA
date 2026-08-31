//! Asking what a path is, and removing it.

use std::fs;
use std::path::Path;

use soma_guest::{EntryKind, FileOutcome, FileRequest};
use tempfile::TempDir;

use super::{at, perform};

/// Asks what one path is.
fn status(path: &Path) -> FileOutcome {
    perform(&FileRequest::Exists { path: at(path) })
}

/// Removes one path.
fn remove(path: &Path, recursive: bool) -> FileOutcome {
    perform(&FileRequest::Remove {
        path: at(path),
        recursive,
    })
}

#[test]
fn reports_a_file_as_a_file() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("note");
    fs::write(&path, b"x").expect("seed file");

    assert_eq!(
        status(&path),
        FileOutcome::Status {
            kind: Some(EntryKind::File)
        }
    );
}

#[test]
fn reports_a_directory_as_a_directory() {
    let root = TempDir::new().expect("temporary directory");

    assert_eq!(
        status(root.path()),
        FileOutcome::Status {
            kind: Some(EntryKind::Directory)
        }
    );
}

#[test]
fn reports_an_absent_path_as_absent() {
    let root = TempDir::new().expect("temporary directory");

    assert_eq!(
        status(&root.path().join("nothing")),
        FileOutcome::Status { kind: None }
    );
}

#[test]
fn reports_a_symbolic_link_as_neither_a_file_nor_a_directory() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("link");
    std::os::unix::fs::symlink("gone", &path).expect("seed link");

    // The link is reported rather than followed, so a link to nothing is still something.
    assert_eq!(
        status(&path),
        FileOutcome::Status {
            kind: Some(EntryKind::Other)
        }
    );
}

#[test]
fn removes_a_file() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("note");
    fs::write(&path, b"x").expect("seed file");

    assert_eq!(remove(&path, false), FileOutcome::Done);
    assert!(!path.exists());
}

#[test]
fn removes_an_empty_directory_without_being_asked_to_recurse() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("empty");
    fs::create_dir(&path).expect("seed directory");

    assert_eq!(remove(&path, false), FileOutcome::Done);
    assert!(!path.exists());
}

#[test]
fn removes_a_populated_directory_when_asked_to_recurse() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("tree");
    fs::create_dir_all(path.join("inner")).expect("seed tree");
    fs::write(path.join("inner/note"), b"x").expect("seed file");

    assert_eq!(remove(&path, true), FileOutcome::Done);
    assert!(!path.exists());
}

#[test]
fn removes_the_link_and_never_what_it_points_at() {
    let root = TempDir::new().expect("temporary directory");
    let target = root.path().join("note");
    let link = root.path().join("link");
    fs::write(&target, b"x").expect("seed file");
    std::os::unix::fs::symlink(&target, &link).expect("seed link");

    assert_eq!(remove(&link, true), FileOutcome::Done);
    assert!(!link.exists());
    assert!(target.is_file(), "the target outlives the link");
}
