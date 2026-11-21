//! Opens only the volume the host's volume store already certified for this launch.

use std::{
    fs::{File, OpenOptions},
    num::NonZeroU64,
    os::unix::fs::OpenOptionsExt as _,
};

use soma::BackendFailureKind;

use super::jailed::Launching;

pub(super) fn open(
    launch: &Launching<'_>,
) -> Result<(Option<File>, Option<NonZeroU64>), BackendFailureKind> {
    let persistence = &launch.prepared.manifest.persistence;
    let volume = match (persistence.enabled, launch.volume) {
        (false, None) => return Ok((None, None)),
        (true, Some(volume)) if launch.checkpoint.is_none() => volume,
        _ => return Err(BackendFailureKind::WorkloadRejected),
    };
    let capacity = NonZeroU64::new(u64::from(persistence.size_class_gib) * 1024 * 1024 * 1024)
        .ok_or(BackendFailureKind::WorkloadRejected)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&volume.path)
        .map_err(|_| BackendFailureKind::Unavailable)?;
    let info = file
        .metadata()
        .map_err(|_| BackendFailureKind::Unavailable)?;
    if !info.is_file() || info.len() != capacity.get() {
        return Err(BackendFailureKind::WorkloadRejected);
    }
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            return Err(BackendFailureKind::ResourceConflict);
        }
        Err(std::fs::TryLockError::Error(_)) => {
            return Err(BackendFailureKind::Unavailable);
        }
    }
    Ok((Some(file), Some(capacity)))
}
