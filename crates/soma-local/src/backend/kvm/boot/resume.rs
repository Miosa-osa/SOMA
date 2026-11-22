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
/// Returns [`BackendFailureKind::WorkloadRejected`] when the members were taken from another
/// Generation and [`BackendFailureKind::Unavailable`] when one is missing or unreadable; the
/// directory is removed either way.
pub(super) fn open(
    directory: &Path,
    generation: [u8; 32],
    overlay: bool,
) -> Result<Members, BackendFailureKind> {
    // Members captured over one Generation's root must never run over another's.
    let named = std::fs::read_to_string(directory.join(crate::RESUME_GENERATION_FILE))
        .map_err(|_| BackendFailureKind::Unavailable)?;
    let expected: String = generation
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let opened = if named.trim() == expected {
        open_members(directory, overlay)
    } else {
        Err(BackendFailureKind::WorkloadRejected)
    };
    let _ignored = std::fs::remove_dir_all(directory);
    opened
}

fn open_members(directory: &Path, overlay: bool) -> Result<Members, BackendFailureKind> {
    let read =
        |name: &str| File::open(directory.join(name)).map_err(|_| BackendFailureKind::Unavailable);
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
