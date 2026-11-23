//! Verify and lock the exact backing inode before changing volume metadata.

use std::{
    fs::{File, OpenOptions, TryLockError},
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::Path,
};

use super::{IO_FAILURE, VolumeError};

pub(super) fn private_file(path: &Path, writable: bool) -> Result<File, VolumeError> {
    let file = open_regular(path, writable)?;
    if file.metadata().map_err(|_| IO_FAILURE)?.mode() & 0o077 != 0 {
        return Err(IO_FAILURE);
    }
    Ok(file)
}

// Existing backing files may predate private mode on the file. The store supplies their paths,
// and links to retained files stay inside the per-operation private directory.
pub(super) fn open_regular(path: &Path, writable: bool) -> Result<File, VolumeError> {
    let file = OpenOptions::new()
        .read(true)
        .write(writable)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| IO_FAILURE)?;
    let meta = file.metadata().map_err(|_| IO_FAILURE)?;
    if !meta.is_file() {
        return Err(IO_FAILURE);
    }
    Ok(file)
}

pub(super) fn inode(file: &File) -> Result<u64, VolumeError> {
    file.metadata()
        .map(|meta| meta.ino())
        .map_err(|_| IO_FAILURE)
}

pub(super) fn lock(file: &File) -> Result<(), VolumeError> {
    match file.try_lock() {
        Ok(()) => Ok(()),
        Err(TryLockError::WouldBlock) => Err(VolumeError::Busy("the workspace is still in use")),
        Err(TryLockError::Error(_)) => Err(IO_FAILURE),
    }
}

pub(super) fn sync(directory: &Path) -> Result<(), VolumeError> {
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|_| IO_FAILURE)
}
