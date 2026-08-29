use std::{
    io::{Read, Seek as _, SeekFrom},
    path::Path,
};

use cap_fs_ext::{DirExt as _, FollowSymlinks, OpenOptionsFollowExt as _};
#[cfg(windows)]
use cap_std::fs::OpenOptionsExt as _;
use cap_std::fs::{Dir, File, OpenOptions};

use crate::{ImportError, ImportErrorKind, ImportPhase, digest, oci::Descriptor, root};

#[cfg(test)]
mod copy_tests;
mod staged;

pub(crate) use staged::StagedObject;

/// A content-addressed store rooted in a private writer authority.
///
/// Portable Rust cannot hard-link an already-open file handle, so publication
/// links a staged name and then verifies the new destination before success.
/// An untrusted writer in `v1/tmp` can cause a failed import, while excluding
/// post-verification mutation through a retained writable handle and preserving
/// availability requires exclusive write authority over the whole store root.
pub(crate) struct Store {
    blobs: Dir,
    temporary: Dir,
}

impl Store {
    pub(crate) fn open(path: &Path) -> Result<Self, ImportError> {
        let root = root::open_existing(path, ImportPhase::Publish, ImportErrorKind::StoreConflict)?;
        let version = ensure_dir(&root, "v1")?;
        let blob_root = ensure_dir(&version, "blobs")?;
        let blobs = ensure_dir(&blob_root, "sha256")?;
        let temporary = ensure_dir(&version, "tmp")?;
        Ok(Self { blobs, temporary })
    }

    pub(crate) fn put_descriptor(
        &self,
        source: &mut impl Read,
        descriptor: &Descriptor,
        maximum: u64,
        phase: ImportPhase,
    ) -> Result<(), ImportError> {
        self.stage_descriptor(source, descriptor, maximum, phase)?
            .publish()
    }

    pub(crate) fn put_bytes(
        &self,
        bytes: &[u8],
        media_type: &str,
        phase: ImportPhase,
    ) -> Result<Descriptor, ImportError> {
        let descriptor = Descriptor {
            media_type: media_type.to_owned(),
            digest: digest::bytes(bytes),
            size: u64::try_from(bytes.len())
                .map_err(|_| ImportError::new(phase, ImportErrorKind::LimitExceeded))?,
            platform: None,
        };
        self.put_descriptor(&mut &*bytes, &descriptor, descriptor.size, phase)?;
        Ok(descriptor)
    }

    pub(crate) fn open_blob(
        &self,
        descriptor: &Descriptor,
        phase: ImportPhase,
    ) -> Result<File, ImportError> {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        self.open_blob_with(descriptor, phase, &options)
    }

    fn open_blob_with(
        &self,
        descriptor: &Descriptor,
        phase: ImportPhase,
        options: &OpenOptions,
    ) -> Result<File, ImportError> {
        let file = self
            .blobs
            .open_with(digest::hex(&descriptor.digest), options)
            .map_err(|_| ImportError::new(phase, ImportErrorKind::StoreConflict))?;
        let metadata = file
            .metadata()
            .map_err(|_| ImportError::new(phase, ImportErrorKind::Io))?;
        if !metadata.is_file() || metadata.len() != descriptor.size {
            return Err(ImportError::new(phase, ImportErrorKind::StoreConflict));
        }
        Ok(file)
    }

    pub(super) fn publish_staged(
        &self,
        name: &str,
        staged: File,
        descriptor: &Descriptor,
    ) -> Result<(), ImportError> {
        let destination = digest::hex(&descriptor.digest);
        let linked = self.temporary.hard_link(name, &self.blobs, destination);
        drop(staged);
        let _ = self.temporary.remove_file(name);
        match linked {
            Ok(()) => {
                if let Err(error) = self.accept_published(descriptor) {
                    self.remove_created(destination)?;
                    return Err(error);
                }
                sync_dir(&self.blobs)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.accept_published(descriptor)
            }
            Err(_) => Err(ImportError::new(ImportPhase::Publish, ImportErrorKind::Io)),
        }
    }

    fn remove_created(&self, name: &str) -> Result<(), ImportError> {
        self.blobs
            .remove_file(name)
            .map_err(|_| ImportError::new(ImportPhase::Publish, ImportErrorKind::StoreConflict))?;
        sync_dir(&self.blobs)
    }

    fn verify_existing(&self, descriptor: &Descriptor) -> Result<(), ImportError> {
        let mut file = self.open_blob(descriptor, ImportPhase::Publish)?;
        file.seek(SeekFrom::Start(0))
            .map_err(|_| ImportError::new(ImportPhase::Publish, ImportErrorKind::Io))?;
        let (actual, size) = digest::reader(&mut file, descriptor.size, ImportPhase::Publish)?;
        if actual != descriptor.digest || size != descriptor.size {
            return Err(ImportError::new(
                ImportPhase::Publish,
                ImportErrorKind::StoreConflict,
            ));
        }
        Ok(())
    }

    fn accept_published(&self, descriptor: &Descriptor) -> Result<(), ImportError> {
        self.verify_existing(descriptor)?;
        self.set_published_readonly(descriptor)?;
        self.verify_existing(descriptor)
    }

    #[cfg(windows)]
    fn set_published_readonly(&self, descriptor: &Descriptor) -> Result<(), ImportError> {
        const FILE_WRITE_ATTRIBUTES: u32 = 0x0000_0100;
        let mut options = OpenOptions::new();
        options
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .follow(FollowSymlinks::No);
        let file = self.open_blob_with(descriptor, ImportPhase::Publish, &options)?;
        let mut permissions = file
            .metadata()
            .map_err(|_| ImportError::new(ImportPhase::Publish, ImportErrorKind::Io))?
            .permissions();
        permissions.set_readonly(true);
        file.set_permissions(permissions)
            .map_err(|_| ImportError::new(ImportPhase::Publish, ImportErrorKind::Io))
    }

    #[cfg(not(windows))]
    fn set_published_readonly(&self, descriptor: &Descriptor) -> Result<(), ImportError> {
        let file = self.open_blob(descriptor, ImportPhase::Publish)?;
        let mut permissions = file
            .metadata()
            .map_err(|_| ImportError::new(ImportPhase::Publish, ImportErrorKind::Io))?
            .permissions();
        permissions.set_readonly(true);
        file.set_permissions(permissions)
            .map_err(|_| ImportError::new(ImportPhase::Publish, ImportErrorKind::Io))
    }
}

fn ensure_dir(parent: &Dir, name: &str) -> Result<Dir, ImportError> {
    match parent.create_dir(name) {
        Ok(()) => sync_dir(parent)?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(ImportError::new(ImportPhase::Publish, ImportErrorKind::Io)),
    }
    parent
        .open_dir_nofollow(name)
        .map_err(|_| ImportError::new(ImportPhase::Publish, ImportErrorKind::StoreConflict))
}

#[cfg(not(windows))]
fn sync_dir(directory: &Dir) -> Result<(), ImportError> {
    directory
        .open(".")
        .and_then(|file| file.sync_all())
        .map_err(|_| ImportError::new(ImportPhase::Publish, ImportErrorKind::Io))
}

#[cfg(windows)]
fn sync_dir(directory: &Dir) -> Result<(), ImportError> {
    // Native Windows has no portable equivalent of fsync on an opened directory.
    directory
        .dir_metadata()
        .map(|_| ())
        .map_err(|_| ImportError::new(ImportPhase::Publish, ImportErrorKind::Io))
}
