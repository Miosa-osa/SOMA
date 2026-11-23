//! Private recovery staging and cleanup after an interrupted copy.

use std::{
    fs::{self, DirBuilder, DirEntry, File, OpenOptions, TryLockError},
    io::{self, ErrorKind, Read as _, Write as _},
    os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _},
    path::{Path, PathBuf},
};

use crate::VolumeError;

use super::IO_FAILURE;

const PREFIX: &str = ".checkpoint-";
const LEASE: &str = ".copy-lease";
const MARKER: &[u8] = b"soma-checkpoint-volume-copy-v1";

pub(super) struct Staging {
    pub(super) path: PathBuf,
    _lease: File,
}

impl Staging {
    pub(super) fn new(root: &Path) -> Result<Self, VolumeError> {
        let mut random = [0_u8; 16];
        getrandom::getrandom(&mut random).map_err(|_| IO_FAILURE)?;
        let path = root.join(format!("{PREFIX}{:032x}", u128::from_be_bytes(random)));
        DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|_| IO_FAILURE)?;
        let lease = acquire(&path).inspect_err(|_| {
            let _ignored = fs::remove_dir_all(&path);
        })?;
        Ok(Self {
            path,
            _lease: lease,
        })
    }

    pub(super) fn write(&self, name: &str, bytes: &[u8]) -> Result<(), VolumeError> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.path.join(name))
            .map_err(|_| IO_FAILURE)?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| IO_FAILURE)
    }

    pub(super) fn publish(&self, destination: &Path) -> Result<(), VolumeError> {
        File::open(&self.path)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| IO_FAILURE)?;
        fs::rename(&self.path, destination).map_err(|_| IO_FAILURE)?;
        let _ignored = fs::remove_file(destination.join(LEASE));
        Ok(())
    }
}

fn acquire(path: &Path) -> Result<File, VolumeError> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path.join(LEASE))
        .map_err(|_| IO_FAILURE)?;
    file.try_lock().map_err(|_| IO_FAILURE)?;
    file.write_all(MARKER)
        .and_then(|()| file.sync_all())
        .map_err(|_| IO_FAILURE)?;
    Ok(file)
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.path);
    }
}

/// Removes exactly the stage created by an earlier daemon, if no process still owns it.
pub(in crate::volume::store) fn remove_interrupted_stage(entry: &DirEntry) -> io::Result<()> {
    let name = entry.file_name();
    let Some(suffix) = name.to_str().and_then(|name| name.strip_prefix(PREFIX)) else {
        return Ok(());
    };
    if suffix.len() != 32
        || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !entry.file_type()?.is_dir()
    {
        return Ok(());
    }
    let path = entry.path();
    let mut lease = match OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path.join(LEASE))
    {
        Ok(file) if file.metadata()?.is_file() => file,
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    match lease.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(()),
        Err(TryLockError::Error(error)) => return Err(error),
    }
    let mut marker = Vec::new();
    (&mut lease)
        .take(MARKER.len() as u64 + 1)
        .read_to_end(&mut marker)?;
    if marker != MARKER {
        return Ok(());
    }
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_removes_only_an_unlocked_copy_with_the_expected_marker() {
        let root = tempfile::tempdir().unwrap();
        let active = Staging::new(root.path()).unwrap();
        let interrupted = root.path().join(format!("{PREFIX}{:032x}", 1));
        let unmarked = root.path().join(format!("{PREFIX}{:032x}", 2));
        fs::create_dir(&interrupted).unwrap();
        fs::create_dir(&unmarked).unwrap();
        fs::write(interrupted.join(LEASE), MARKER).unwrap();
        fs::write(interrupted.join(super::super::VOLUME_FILE), b"partial").unwrap();
        for entry in fs::read_dir(root.path()).unwrap() {
            remove_interrupted_stage(&entry.unwrap()).unwrap();
        }
        assert!(active.path.is_dir());
        assert!(unmarked.is_dir());
        assert!(!interrupted.exists());
    }
}
