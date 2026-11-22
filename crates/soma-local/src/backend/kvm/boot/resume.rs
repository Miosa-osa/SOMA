//! Opening a decrypted Suspension's members for a resume (ADR 0034).
//!
//! The members are plaintext only between the store's open and this one: each is opened, then
//! the directory is removed, so the machine holds the only references and nothing on the
//! filesystem names them once the launch has them. The sealed Suspension is untouched, so a
//! launch that fails after this point can be retried from it.

use std::fs::{File, OpenOptions};
use std::path::Path;

use soma::BackendFailureKind;

/// The three members, opened.
pub(super) struct Members {
    pub(super) memory: File,
    pub(super) state: File,
    /// The private head, writable, present when the Generation declares writable storage.
    pub(super) overlay: Option<File>,
}

/// Opens the members in `directory` and removes it.
///
/// # Errors
///
/// Returns [`BackendFailureKind::Unavailable`] when a member is missing or unreadable; the
/// directory is removed either way.
pub(super) fn open(directory: &Path, overlay: bool) -> Result<Members, BackendFailureKind> {
    let opened = open_members(directory, overlay);
    let _ignored = std::fs::remove_dir_all(directory);
    opened
}

fn open_members(directory: &Path, overlay: bool) -> Result<Members, BackendFailureKind> {
    let read = |name: &str| File::open(directory.join(name)).map_err(|_| BackendFailureKind::Unavailable);
    let memory = read("memory.raw")?;
    let state = read("state.somasnap")?;
    let overlay = overlay
        .then(|| {
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(directory.join("overlay.raw"))
                .map_err(|_| BackendFailureKind::Unavailable)
        })
        .transpose()?;
    Ok(Members {
        memory,
        state,
        overlay,
    })
}
