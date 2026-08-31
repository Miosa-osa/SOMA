//! One real filesystem error reduced to one of the six causes the protocol admits.
//!
//! The classification is driven by the errno rather than by `io::ErrorKind` because the kernel
//! distinguishes exactly the cases the protocol names, while the standard library folds several
//! of them into one kind on some releases and splits them on others.
//!
//! Everything the kernel can report that is not one of the five named causes becomes `Failed`.
//! That is the honest answer: the operation did not happen, and the reason is the guest's own
//! business rather than the caller's.

use std::io;

use soma_guest::{FileFailure, FileOutcome};

/// Reduces one error to the cause the caller is allowed to see.
pub(super) fn classify(error: &io::Error) -> FileFailure {
    match error.raw_os_error() {
        Some(libc::ENOENT) => FileFailure::NotFound,
        Some(libc::EACCES | libc::EPERM | libc::EROFS) => FileFailure::Denied,
        Some(libc::EISDIR | libc::ENOTDIR) => FileFailure::WrongKind,
        Some(libc::EEXIST) => FileFailure::Exists,
        Some(libc::ENOTEMPTY) => FileFailure::NotEmpty,
        _ => FileFailure::Failed,
    }
}

/// Answers an operation that has nothing to report beyond whether it happened.
pub(super) fn done(result: io::Result<()>) -> FileOutcome {
    match result {
        Ok(()) => FileOutcome::Done,
        Err(error) => failed(&error),
    }
}

/// Answers an operation that did not happen.
pub(super) fn failed(error: &io::Error) -> FileOutcome {
    FileOutcome::Failed(classify(error))
}
