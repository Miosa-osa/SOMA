//! Publish a staged replacement over a workspace backing, and roll a published one back.

use std::{
    fs::{self, File},
    io::ErrorKind,
    os::unix::fs::MetadataExt as _,
    path::Path,
};

use super::{
    DISPLACED, IO_FAILURE, Intent, RETAINED, ROLLBACK, VOLUME_FILE, VolumeError, VolumeState,
    file::{inode, lock, open_regular, private_file, sync},
};

pub(super) fn verify_stage(
    path: &Path,
    intent: Intent,
    original: u64,
    transition: u64,
    class_gib: u32,
) -> Result<(), VolumeError> {
    if original != intent.original || transition != intent.transition {
        return Err(VolumeError::ChangedIntent);
    }
    let staged = private_file(&path.join(VOLUME_FILE), true)?;
    let meta = staged.metadata().map_err(|_| IO_FAILURE)?;
    if meta.ino() != intent.replacement || meta.len() != u64::from(class_gib) << 30 {
        return Err(VolumeError::ChangedIntent);
    }
    lock(&staged)
}

pub(super) fn publish(
    directory: &Path,
    path: &Path,
    backing: &Path,
    intent: Intent,
    state: VolumeState,
    transition: u64,
) -> Result<(), VolumeError> {
    let current = open_regular(backing, true)?;
    let current_inode = inode(&current)?;
    let retained = path.join(RETAINED);
    if current_inode == intent.replacement {
        if inode(&open_regular(&retained, false)?)? != intent.original {
            return Err(VolumeError::ChangedIntent);
        }
        sync(directory)?;
        return sync(path);
    }
    if current_inode != intent.original
        || transition != intent.transition
        || state != VolumeState::Detached
    {
        return Err(VolumeError::ChangedIntent);
    }
    lock(&current)?;
    let staged_path = path.join(VOLUME_FILE);
    let staged = private_file(&staged_path, true)?;
    if inode(&staged)? != intent.replacement {
        return Err(VolumeError::ChangedIntent);
    }
    require_worker_lock(&staged)?;
    retain_link(backing, &retained, intent.original)?;
    sync(path)?;
    fs::rename(&staged_path, backing).map_err(|_| IO_FAILURE)?;
    sync(directory)?;
    sync(path)
}

pub(super) fn rollback(
    directory: &Path,
    path: &Path,
    backing: &Path,
    intent: Intent,
) -> Result<(), VolumeError> {
    if path.join(VOLUME_FILE).exists() {
        return Err(VolumeError::Busy(
            "the workspace restore has not been published",
        ));
    }
    let previous_path = path.join(RETAINED);
    let previous = open_regular(&previous_path, true)?;
    if inode(&previous)? != intent.original {
        return Err(VolumeError::ChangedIntent);
    }
    let current = open_regular(backing, true)?;
    lock(&current)?;
    let displaced = path.join(DISPLACED);
    match inode(&current)? {
        current_inode if current_inode == intent.original => {
            if inode(&private_file(&displaced, false)?)? != intent.replacement {
                return Err(VolumeError::ChangedIntent);
            }
            sync(directory)?;
            return sync(path);
        }
        current_inode if current_inode == intent.replacement => {}
        _ => return Err(VolumeError::ChangedIntent),
    }

    lock(&previous)?;
    retain_link(backing, &displaced, intent.replacement)?;
    let restored = path.join(ROLLBACK);
    retain_link(&previous_path, &restored, intent.original)?;
    sync(path)?;
    fs::rename(&restored, backing).map_err(|_| IO_FAILURE)?;
    sync(directory)?;
    sync(path)
}

pub(super) fn require_worker_lock(staged: &File) -> Result<(), VolumeError> {
    match staged.try_lock() {
        Err(fs::TryLockError::WouldBlock) => Ok(()),
        Ok(()) => Err(VolumeError::Busy(
            "the recovered worker does not own its workspace",
        )),
        Err(fs::TryLockError::Error(_)) => Err(IO_FAILURE),
    }
}

pub(super) fn retain_link(
    source: &Path,
    destination: &Path,
    expected: u64,
) -> Result<(), VolumeError> {
    match fs::hard_link(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            if inode(&open_regular(destination, false)?)? != expected {
                return Err(VolumeError::ChangedIntent);
            }
            Ok(())
        }
        Err(_) => Err(IO_FAILURE),
    }
}
