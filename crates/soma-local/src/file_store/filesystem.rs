use std::{
    fs::{self, File, OpenOptions},
    io,
    path::Path,
};

use soma::StateStoreFailure;

use super::failure::{corrupt, unavailable};

pub(super) fn ensure_directory(path: &Path) -> Result<(), StateStoreFailure> {
    if !existing_directory(path)? {
        fs::create_dir_all(path).map_err(|_| unavailable())?;
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| unavailable())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(corrupt());
    }
    set_directory_permissions(path)?;
    Ok(())
}

pub(super) fn existing_directory(path: &Path) -> Result<bool, StateStoreFailure> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(corrupt()),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(unavailable()),
    }
}

pub(super) fn reject_unsafe_existing_file(path: &Path) -> Result<(), StateStoreFailure> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(corrupt()),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(unavailable()),
    }
}

#[cfg(unix)]
pub(super) fn require_single_link(path: &Path) -> Result<(), StateStoreFailure> {
    let metadata = fs::symlink_metadata(path).map_err(|_| unavailable())?;
    require_single_link_metadata(&metadata)
}

#[cfg(not(unix))]
pub(super) fn require_single_link(path: &Path) -> Result<(), StateStoreFailure> {
    fs::symlink_metadata(path)
        .map(|_| ())
        .map_err(|_| unavailable())
}

#[cfg(unix)]
pub(super) fn require_single_link_metadata(
    metadata: &fs::Metadata,
) -> Result<(), StateStoreFailure> {
    use std::os::unix::fs::MetadataExt as _;

    if metadata.nlink() == 1 {
        Ok(())
    } else {
        Err(corrupt())
    }
}

#[cfg(not(unix))]
pub(super) fn require_single_link_metadata(
    metadata: &fs::Metadata,
) -> Result<(), StateStoreFailure> {
    if metadata.is_file() {
        Ok(())
    } else {
        Err(corrupt())
    }
}

/// The permission bits of a mode, without the file type and the set-user bits.
#[cfg(unix)]
const PERMISSION_BITS: u32 = 0o7777;

#[cfg(unix)]
pub(super) fn set_directory_permissions(path: &Path) -> Result<(), StateStoreFailure> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    // Every state operation passes through here, on a directory that is almost always already
    // owner-private. Rewriting a mode that already holds dirties the inode and puts it in the
    // filesystem's next commit, which is the cost this read avoids paying on the common path.
    let metadata = fs::metadata(path).map_err(|_| unavailable())?;
    if metadata.mode() & PERMISSION_BITS == 0o700 {
        return Ok(());
    }
    File::open(path)
        .and_then(|directory| directory.set_permissions(fs::Permissions::from_mode(0o700)))
        .map_err(|_| unavailable())
}

#[cfg(not(unix))]
pub(super) fn set_directory_permissions(path: &Path) -> Result<(), StateStoreFailure> {
    fs::metadata(path).map(|_| ()).map_err(|_| unavailable())
}

#[cfg(unix)]
pub(super) fn set_file_permissions(file: &File) -> Result<(), StateStoreFailure> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    // The shard lock is opened and this is called on it once per state operation, by every
    // process sharing the store. The mode it is being set to is the mode it was created with.
    let metadata = file.metadata().map_err(|_| unavailable())?;
    if metadata.mode() & PERMISSION_BITS == 0o600 {
        return Ok(());
    }
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| unavailable())
}

#[cfg(not(unix))]
pub(super) fn set_file_permissions(file: &File) -> Result<(), StateStoreFailure> {
    file.metadata().map(|_| ()).map_err(|_| unavailable())
}

#[cfg(unix)]
pub(super) fn set_create_file_mode(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt as _;

    options.mode(0o600);
}

#[cfg(not(unix))]
pub(super) const fn set_create_file_mode(_options: &mut OpenOptions) {}

#[cfg(unix)]
pub(super) fn sync_directory(directory: &Path) -> Result<(), StateStoreFailure> {
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|_| unavailable())
}

#[cfg(not(unix))]
pub(super) fn sync_directory(directory: &Path) -> Result<(), StateStoreFailure> {
    fs::metadata(directory)
        .map(|_| ())
        .map_err(|_| unavailable())
}
