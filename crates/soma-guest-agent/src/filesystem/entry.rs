//! What one path is, and removing it.
//!
//! Both operations look at the path itself and never follow a final symbolic link. A caller that
//! asks what a path is wants the link, not its target, because the target may not exist and the
//! two answers differ; a caller that asks to remove a link means the link.

use std::fs::{self, FileType};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use soma_guest::{EntryKind, FileOutcome};

use super::failure;

/// Reports what the path is, or that nothing is there.
pub(super) fn status(path: &Path) -> FileOutcome {
    match fs::symlink_metadata(path) {
        Ok(metadata) => FileOutcome::Status {
            kind: Some(kind_of(metadata.file_type())),
        },
        // Absence is the answer to this question rather than a failure of it, which is why the
        // outcome carries an optional kind at all.
        Err(error) if error.raw_os_error() == Some(libc::ENOENT) => {
            FileOutcome::Status { kind: None }
        }
        Err(error) => failure::failed(&error),
    }
}

/// Reports kind, size, permission bits, and modification time of the path itself.
pub(super) fn stat(path: &Path) -> FileOutcome {
    match fs::symlink_metadata(path) {
        Ok(metadata) => FileOutcome::Stat {
            kind: kind_of(metadata.file_type()),
            size: if metadata.is_file() {
                metadata.len()
            } else {
                0
            },
            mode: metadata.mode() & 0o777,
            modified: u64::try_from(metadata.mtime()).unwrap_or(0),
        },
        Err(error) => failure::failed(&error),
    }
}

/// Moves `from` to `to`, atomically replacing a file already at `to`.
///
/// The kernel refuses a rename that would put a directory over a file or the reverse, and that
/// refusal is reported as the wrong kind rather than as a generic failure.
pub(super) fn rename(from: &Path, to: &Path) -> FileOutcome {
    failure::done(fs::rename(from, to))
}

/// Removes the path, taking a directory's contents with it only when asked.
///
/// The kind is read first so that a directory refused for being non-empty reports exactly that
/// rather than the kernel's `EISDIR` from an attempted file unlink.
pub(super) fn remove(path: &Path, recursive: bool) -> FileOutcome {
    let file_type = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata.file_type(),
        Err(error) => return failure::failed(&error),
    };
    let removed = if file_type.is_dir() {
        if recursive {
            fs::remove_dir_all(path)
        } else {
            fs::remove_dir(path)
        }
    } else {
        fs::remove_file(path)
    };
    failure::done(removed)
}

/// Names one entry's kind, folding everything that is neither a file nor a directory into the
/// one kind the protocol keeps for them.
pub(super) fn kind_of(file_type: FileType) -> EntryKind {
    if file_type.is_file() {
        EntryKind::File
    } else if file_type.is_dir() {
        EntryKind::Directory
    } else {
        EntryKind::Other
    }
}
