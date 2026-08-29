use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};

use soma::StateStoreFailure;

use super::{
    failure::{corrupt, unavailable},
    filesystem::{reject_unsafe_existing_file, set_create_file_mode, set_file_permissions},
};

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
