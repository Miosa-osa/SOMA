//! Creating and listing directories, including the paging a bounded listing forces on a caller.

use std::fs;
use std::path::Path;

use soma_guest::{DirectoryEntry, EntryKind, FileOutcome, FileRequest, MAX_ENTRIES};
use tempfile::TempDir;

use super::{at, perform};

/// Lists one directory from an entry offset.
fn list(path: &Path, offset: u32) -> FileOutcome {
    perform(&FileRequest::ReadDirectory {
        path: at(path),
        offset,
    })
}

/// Returns the entries of a `Listed` outcome and whether more remain.
fn listed(outcome: &FileOutcome) -> (Vec<DirectoryEntry>, bool) {
    match outcome {
        FileOutcome::Listed { entries, more } => (entries.clone(), *more),
        other => panic!("expected a listing outcome, got {other:?}"),
    }
}

/// Sorts entry names so an assertion does not depend on the kernel's ordering.
fn names(entries: &[DirectoryEntry]) -> Vec<Vec<u8>> {
    let mut names: Vec<Vec<u8>> = entries.iter().map(|entry| entry.name.to_vec()).collect();
    names.sort_unstable();
    names
}

#[test]
fn makes_one_directory() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("made");

    let outcome = perform(&FileRequest::MakeDirectory {
        path: at(&path),
        parents: false,
    });

    assert_eq!(outcome, FileOutcome::Done);
    assert!(path.is_dir());
}

#[test]
fn makes_missing_parents_only_when_asked() {
    let root = TempDir::new().expect("temporary directory");
    let path = root.path().join("one/two/three");

    let refused = perform(&FileRequest::MakeDirectory {
        path: at(&path),
        parents: false,
    });
    let made = perform(&FileRequest::MakeDirectory {
        path: at(&path),
        parents: true,
    });

    assert_eq!(
        refused,
        FileOutcome::Failed(soma_guest::FileFailure::NotFound)
    );
    assert_eq!(made, FileOutcome::Done);
    assert!(path.is_dir());
}

#[test]
fn asking_for_parents_accepts_a_directory_that_is_already_there() {
    let root = TempDir::new().expect("temporary directory");

    let outcome = perform(&FileRequest::MakeDirectory {
        path: at(root.path()),
        parents: true,
    });

    assert_eq!(outcome, FileOutcome::Done);
}

#[test]
fn lists_a_directory_with_the_kind_of_every_entry() {
    let root = TempDir::new().expect("temporary directory");
    fs::write(root.path().join("file"), b"x").expect("seed file");
    fs::create_dir(root.path().join("dir")).expect("seed directory");
    std::os::unix::fs::symlink("file", root.path().join("link")).expect("seed link");

    let (entries, more) = listed(&list(root.path(), 0));

    assert!(!more);
    assert_eq!(
        names(&entries),
        vec![b"dir".to_vec(), b"file".to_vec(), b"link".to_vec()]
    );
    let kind_of = |wanted: &[u8]| {
        entries
            .iter()
            .find(|entry| &*entry.name == wanted)
            .expect("entry present")
            .kind
    };
    assert_eq!(kind_of(b"file"), EntryKind::File);
    assert_eq!(kind_of(b"dir"), EntryKind::Directory);
    assert_eq!(kind_of(b"link"), EntryKind::Other);
}

#[test]
fn a_listing_that_skips_entries_returns_only_the_rest() {
    let root = TempDir::new().expect("temporary directory");
    for index in 0..4 {
        fs::write(root.path().join(format!("f{index}")), b"x").expect("seed file");
    }

    let (entries, more) = listed(&list(root.path(), 3));

    assert_eq!(entries.len(), 1);
    assert!(!more);
}

#[test]
fn pages_a_directory_larger_than_one_listing() {
    let root = TempDir::new().expect("temporary directory");
    let total = MAX_ENTRIES + 5;
    for index in 0..total {
        fs::write(root.path().join(format!("f{index:05}")), b"x").expect("seed file");
    }

    let (first, first_more) = listed(&list(root.path(), 0));
    let offset = u32::try_from(first.len()).expect("a bounded page length");
    let (second, second_more) = listed(&list(root.path(), offset));

    assert_eq!(first.len(), MAX_ENTRIES);
    assert!(
        first_more,
        "the directory holds more than one listing carries"
    );
    assert_eq!(second.len(), total - MAX_ENTRIES);
    assert!(!second_more, "the second page is the last one");
}

#[test]
fn every_entry_of_a_paged_directory_is_seen_exactly_once() {
    let root = TempDir::new().expect("temporary directory");
    let total = MAX_ENTRIES + 5;
    for index in 0..total {
        fs::write(root.path().join(format!("f{index:05}")), b"x").expect("seed file");
    }

    let (first, _) = listed(&list(root.path(), 0));
    let offset = u32::try_from(first.len()).expect("a bounded page length");
    let (second, _) = listed(&list(root.path(), offset));
    let mut seen = names(&first);
    seen.extend(names(&second));
    seen.sort_unstable();
    seen.dedup();

    assert_eq!(seen.len(), total);
}

#[test]
fn an_empty_directory_lists_nothing() {
    let root = TempDir::new().expect("temporary directory");

    let (entries, more) = listed(&list(root.path(), 0));

    assert!(entries.is_empty());
    assert!(!more);
}
