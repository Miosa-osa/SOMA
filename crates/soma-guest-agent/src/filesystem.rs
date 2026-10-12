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

/// The virtual directory a host reads `<prefix><nonce>` under to learn the guest API level.
///
/// The host picks a fresh random nonce per session and accepts only an answer that echoes it.
/// A file a tenant planted at a fixed path can therefore never pass for the agent's answer, and
/// an older agent answers the read with an ordinary failure (or file bytes that lack the nonce
/// echo), so no rollout order makes the host send a word the guest would not understand.
/// The probe is answered from memory and never touches the disk.
pub use soma_guest::GUEST_API_PROBE_PREFIX;

/// Longest nonce the probe accepts.
const MAX_NONCE_BYTES: usize = soma_guest::MAX_PROBE_NONCE_BYTES;

/// Performs one decoded filesystem request and returns the outcome that answers it.
#[must_use]
pub fn perform(request: &FileRequest) -> FileOutcome {
    if let FileRequest::Read { path, offset, .. } = request
        && let Some(nonce) = path.strip_prefix(GUEST_API_PROBE_PREFIX)
    {
        return probe(nonce, *offset);
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
        FileRequest::ReadDirectoryDetailed { offset, .. } => {
            directory::list_detailed(&path, *offset)
        }
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

/// Answers a read of the probe path with the level and the nonce, honouring the offset.
fn probe(nonce: &[u8], offset: u64) -> FileOutcome {
    if nonce.is_empty()
        || nonce.len() > MAX_NONCE_BYTES
        || !nonce.iter().all(u8::is_ascii_alphanumeric)
    {
        return FileOutcome::Failed(FileFailure::NotFound);
    }
    let mut text = format!("{GUEST_API_LEVEL} ").into_bytes();
    text.extend_from_slice(nonce);
    text.push(b'\n');
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
