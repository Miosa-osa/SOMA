//! How long an import stage may sit, how many a host keeps open, and how much they may hold.
//!
//! A backup job that crashes leaves its stage behind, and the retention sweep never reads a
//! `.staging-` name, so these bounds are what keep abandoned imports from filling the encrypted
//! checkpoint root.

use std::{
    fs::{self, OpenOptions},
    io::ErrorKind,
    os::{fd::AsFd as _, unix::fs::OpenOptionsExt as _},
    path::Path,
    time::{Duration, SystemTime},
};

use super::{ImportRefusal, PREFIX, stage_names};
use crate::CheckpointFailure;

/// The most imports one host holds open at a time.
pub(super) const MAX_OPEN_IMPORTS: usize = 16;
/// A stage untouched for this long belongs to a transfer that is not coming back.
pub(super) const STAGE_TTL: Duration = Duration::from_hours(24);
/// The most every open import together may hold.
pub(super) const MAX_STAGED_BYTES: u64 = 2 << 40;
/// Free space the checkpoint root keeps after a received artifact, for captures and restores.
pub(super) const FREE_MARGIN_BYTES: u64 = 8 << 30;

/// Removes stages older than [`STAGE_TTL`] that no request holds.
pub(super) fn sweep_expired(root: &Path) {
    let now = SystemTime::now();
    for stage in stages(root) {
        let expired = fs::symlink_metadata(&stage)
            .and_then(|meta| meta.modified())
            .is_ok_and(|modified| {
                now.duration_since(modified)
                    .is_ok_and(|age| age > STAGE_TTL)
            });
        if expired && unheld(&stage) {
            let _ignored = remove_stage(&stage);
        }
    }
}

/// How many stages the root holds.
pub(super) fn open_imports(root: &Path) -> usize {
    stages(root).count()
}

/// Refuses an artifact of `length` bytes that would pass the staged-bytes cap or leave the root
/// with less than [`FREE_MARGIN_BYTES`] free.
pub(super) fn require_room(root: &Path, length: u64) -> Result<(), ImportRefusal> {
    let staged: u64 = stages(root).map(|stage| bytes_in(&stage)).sum();
    if staged.saturating_add(length) > MAX_STAGED_BYTES {
        return Err(ImportRefusal::NoSpace);
    }
    let fs = rustix::fs::statvfs(root).map_err(|_| CheckpointFailure::Storage)?;
    let free = fs.f_bavail.saturating_mul(fs.f_frsize);
    if free < length.saturating_add(FREE_MARGIN_BYTES) {
        return Err(ImportRefusal::NoSpace);
    }
    Ok(())
}

/// Removes every name a stage may hold and the stage itself; an absent stage is already gone.
pub(super) fn remove_stage(stage: &Path) -> Result<(), CheckpointFailure> {
    for name in stage_names() {
        match fs::remove_file(stage.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(_) => return Err(CheckpointFailure::Storage),
        }
    }
    match fs::remove_dir(stage) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(_) => Err(CheckpointFailure::Storage),
    }
}

fn stages(root: &Path) -> impl Iterator<Item = std::path::PathBuf> {
    fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(PREFIX))
        })
        .map(|entry| entry.path())
}

fn bytes_in(stage: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt as _;
    stage_names()
        .filter_map(|name| fs::symlink_metadata(stage.join(name)).ok())
        .map(|meta| meta.blocks().saturating_mul(512))
        .sum()
}

/// Whether no request holds the stage's lock right now.
fn unheld(stage: &Path) -> bool {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(stage)
        .is_ok_and(|directory| {
            rustix::fs::flock(
                directory.as_fd(),
                rustix::fs::FlockOperation::NonBlockingLockExclusive,
            )
            .is_ok()
        })
}
