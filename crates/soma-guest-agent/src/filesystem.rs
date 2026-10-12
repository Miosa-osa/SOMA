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
mod mode;

#[cfg(test)]
mod tests;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use soma_guest::{FileFailure, FileOutcome, FileRequest, MAX_PATH_BYTES};

/// The guest API level this agent speaks, answered at [`GUEST_API_PROBE`].
///
/// Level 1 is the original file protocol. Level 2 adds stat, rename, and ranged operations.
pub const GUEST_API_LEVEL: u8 = 2;

/// The virtual path a host reads to learn the guest API level.
///
/// An agent that predates this answers the read with an ordinary not-found failure, so a host
/// learns the level without sending any word an old guest would not understand, in any rollout
/// order.
pub const GUEST_API_PROBE: &[u8] = b"/.soma/guest-api";

/// Performs one decoded filesystem request and returns the outcome that answers it.
#[must_use]
pub fn perform(request: &FileRequest) -> FileOutcome {
    if let FileRequest::Read { path, offset, .. } = request {
        if path.as_ref() == GUEST_API_PROBE {
            return probe(*offset);
        }
    }
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
        FileRequest::Create { mode, .. } => mode::create(&path, *mode),
        FileRequest::SetMode { mode, .. } => mode::set(&path, *mode),
        FileRequest::Remove { recursive, .. } => entry::remove(&path, *recursive),
        FileRequest::Stat { .. } => entry::stat(&path),
        FileRequest::Rename { to, .. } => match resolve(to) {
            Some(to) => entry::rename(&path, &to),
            None => FileOutcome::Failed(FileFailure::Failed),
        },
    }
}

/// Answers a read of the probe path with the level, honouring the offset like a file would.
fn probe(offset: u64) -> FileOutcome {
    let text = format!("{GUEST_API_LEVEL}\n").into_bytes();
    let start = usize::try_from(offset).map_or(text.len(), |offset| offset.min(text.len()));
    FileOutcome::Read {
        bytes: text[start..].to_vec().into(),
        end: true,
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
