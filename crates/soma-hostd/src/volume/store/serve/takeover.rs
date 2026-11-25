//! Moving a volume to a new Instance once the machine that owned it is proven gone (SOMA#231).
//!
//! A volume is bound to the Instance that first attached it. When that Instance's machine crashed,
//! or was detached and deleted, nothing ever moved the volume on, so the customer's disk was
//! stranded behind a stale attachment. An attach for another Instance of the same tenant now takes
//! the volume over, but only on proof that no machine still uses the file: its exclusive lock,
//! which every live worker holds, is free, and no other process has the file open at all, which
//! also covers a worker started before every launch path took that lock.

use std::{fs, io, path::Path};

use super::super::{VolumeError, VolumeSidecar, VolumeState};
use crate::volume::AttachRequest;

/// The sidecar an attach is admitted against: the volume as it is when this Instance owns it, or
/// as it would be moved to this Instance once its machine is proven gone. The proof's handle is
/// closed before the attach's repair takes the same lock.
///
/// # Errors
///
/// Returns the refusal of [`require_released`].
pub(in crate::volume::store) fn current(
    existing: Option<&VolumeSidecar>,
    request: &AttachRequest,
    path: &Path,
) -> Result<Option<VolumeSidecar>, VolumeError> {
    let Some(existing) = existing else {
        return Ok(None);
    };
    if existing.instance == request.instance || existing.state == VolumeState::Deleting {
        return Ok(Some(existing.clone()));
    }
    require_released(existing, request, path)?;
    Ok(Some(VolumeSidecar {
        instance: request.instance,
        ..existing.clone()
    }))
}

/// Proves the machine that owned `existing` no longer uses the file at `path`.
///
/// # Errors
///
/// Returns `Busy` while anything still holds the volume, or when it belongs to another tenant or
/// the proof cannot be made; nothing is changed.
fn require_released(
    existing: &VolumeSidecar,
    request: &AttachRequest,
    path: &Path,
) -> Result<(), VolumeError> {
    if existing.tenant != request.tenant {
        return Err(VolumeError::Busy("the volume belongs to another Instance"));
    }
    let in_use = VolumeError::Busy("the volume is still in use");
    let Ok(file) = fs::OpenOptions::new().read(true).write(true).open(path) else {
        // No file to hold proves nothing about the machine, so the binding stays where it is.
        return Err(VolumeError::Busy("the volume belongs to another Instance"));
    };
    match file.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) => return Err(in_use),
        Err(fs::TryLockError::Error(_)) => {
            return Err(VolumeError::Ledger("the volume lock could not be checked"));
        }
    }
    match held_elsewhere(&file) {
        Ok(false) => Ok(()),
        Ok(true) => Err(in_use),
        Err(_) => Err(VolumeError::Ledger(
            "the volume's holders could not be checked",
        )),
    }
}

/// Whether any process other than this one has the same file open.
#[cfg(target_os = "linux")]
fn held_elsewhere(file: &fs::File) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt as _;

    let own = file.metadata()?;
    let this = std::process::id().to_string();
    for process in fs::read_dir("/proc")? {
        let process = process?;
        let name = process.file_name();
        let Some(pid) = name
            .to_str()
            .filter(|pid| pid.bytes().all(|b| b.is_ascii_digit()))
        else {
            continue;
        };
        if pid == this {
            continue;
        }
        // A process that exits during the scan, or whose table cannot be read, holds nothing
        // this scan could act on.
        let Ok(descriptors) = fs::read_dir(process.path().join("fd")) else {
            continue;
        };
        for descriptor in descriptors.flatten() {
            if let Ok(target) = fs::metadata(descriptor.path())
                && target.dev() == own.dev()
                && target.ino() == own.ino()
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Without a process table to read, nothing proves the file unused, so a takeover is refused.
#[cfg(not(target_os = "linux"))]
fn held_elsewhere(_file: &fs::File) -> io::Result<bool> {
    Ok(true)
}
