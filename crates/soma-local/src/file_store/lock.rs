use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};

use soma::StateStoreFailure;

use super::{
    failure::{corrupt, unavailable},
    filesystem::{reject_unsafe_existing_file, set_create_file_mode, set_file_permissions},
};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) fn restore_lease(
    state_root: &Path,
    instance: &soma::InstanceId,
) -> Result<File, soma::BackendFailureKind> {
    use soma::BackendFailureKind;

    let locks = state_root.join(super::layout::LOCK_DIRECTORY);
    let shard = &instance.as_str()[..3];
    let file = open_lock(&locks.join(format!("restore-{shard}.lock")))
        .map_err(|_| BackendFailureKind::Unavailable)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(BackendFailureKind::ResourceConflict),
        Err(std::fs::TryLockError::Error(_)) => Err(BackendFailureKind::Unavailable),
    }
}

pub(super) fn open_lock(path: &Path) -> Result<File, StateStoreFailure> {
    reject_unsafe_existing_file(path)?;
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    set_create_file_mode(&mut options);
    let file = options.open(path).map_err(|_| unavailable())?;
    let metadata = fs::symlink_metadata(path).map_err(|_| unavailable())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(corrupt());
    }
    set_file_permissions(&file)?;
    Ok(file)
}
