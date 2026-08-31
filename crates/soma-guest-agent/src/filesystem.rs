//! The guest side of the bounded filesystem protocol.
//!
//! A request arrives already decoded, so its path is bounded, absolute, and free of interior nul
//! bytes before this module sees it. What remains is the part the protocol deliberately left to
//! the agent: turning those bytes into a path on the guest's own filesystem, doing the work, and
//! answering with one outcome.
//!
//! There is no confinement here beyond the machine itself. The sandbox boundary is the virtual
//! machine, so a second policy inside it would only be a second thing to get wrong; the agent
//! reaches whatever the guest's own permissions allow and reports a refusal when they do not.
//!
//! No failure carries an errno or a message. Both would describe the guest to whoever asked, and
//! a message can carry a path, so every error becomes one of the six causes the protocol admits.

mod contents;
mod directory;
mod entry;
mod failure;

#[cfg(test)]
mod tests;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use soma_guest::{FileFailure, FileOutcome, FileRequest, MAX_PATH_BYTES};

/// Performs one decoded filesystem request and returns the outcome that answers it.
#[must_use]
pub fn perform(request: &FileRequest) -> FileOutcome {
    let Some(path) = resolve(request.path()) else {
        return FileOutcome::Failed(FileFailure::Failed);
    };
    match request {
        FileRequest::Read { offset, length, .. } => contents::read(&path, *offset, *length),
        FileRequest::Write {
            offset,
            create,
            shorten,
            bytes,
            ..
        } => contents::write(&path, *offset, *create, *shorten, bytes),
        FileRequest::MakeDirectory { parents, .. } => directory::make(&path, *parents),
        FileRequest::ReadDirectory { offset, .. } => directory::list(&path, *offset),
        FileRequest::Exists { .. } => entry::status(&path),
        FileRequest::Remove { recursive, .. } => entry::remove(&path, *recursive),
    }
}

/// Turns request bytes into a guest path, refusing what the protocol does not carry.
///
/// The decoder already applied these bounds, so this only ever rejects a request built inside
/// the guest. Checking again is two comparisons and keeps the invariant true at the one place
/// that hands the bytes to the kernel, where a nul byte would otherwise end the path early and
/// name a different file than the one the request asked for.
fn resolve(path: &[u8]) -> Option<PathBuf> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES {
        return None;
    }
    if path[0] != b'/' || path.contains(&0) {
        return None;
    }
    Some(PathBuf::from(OsStr::from_bytes(path)))
}
