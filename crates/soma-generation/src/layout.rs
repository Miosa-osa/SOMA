use std::{io::Read as _, path::Path};

use cap_fs_ext::{DirExt as _, FollowSymlinks, OpenOptionsFollowExt as _};
use cap_std::fs::{Dir, File, OpenOptions};

use crate::{
    ImportError, ImportErrorKind, ImportLimits, ImportPhase, OciSelection, digest,
    oci::{Descriptor, LayoutWire, parse_json},
    root,
    traversal::{self, Traversal},
};

const MAX_METADATA_BYTES: u64 = 4 * 1024 * 1024;

pub(crate) struct Layout {
    root: Dir,
    blobs: Dir,
}

impl Layout {
    pub(crate) fn open(path: &Path, maximum: u64) -> Result<Self, ImportError> {
        let root =
            root::open_existing(path, ImportPhase::OpenLayout, ImportErrorKind::InvalidInput)?;
        let blob_root = root.open_dir_nofollow("blobs").map_err(|_| {
            ImportError::new(ImportPhase::OpenLayout, ImportErrorKind::InvalidInput)
        })?;
        let blobs = blob_root.open_dir_nofollow("sha256").map_err(|_| {
            ImportError::new(ImportPhase::OpenLayout, ImportErrorKind::InvalidInput)
        })?;
        let layout_bytes = read_named(
            &root,
            "oci-layout",
            MAX_METADATA_BYTES.min(maximum),
            ImportPhase::OpenLayout,
        )?;
        let marker: LayoutWire = parse_json(&layout_bytes, ImportPhase::OpenLayout)?;
        if marker.version != "1.0.0" {
            return Err(ImportError::new(
                ImportPhase::OpenLayout,
                ImportErrorKind::Unsupported,
            ));
        }
        Ok(Self { root, blobs })
    }

    pub(crate) fn traverse(
        &self,
        selection: OciSelection<'_>,
        limits: ImportLimits,
    ) -> Result<Traversal, ImportError> {
        traversal::traverse(self, selection, limits)
    }

    pub(crate) fn read_top_index(&self, maximum: u64) -> Result<Vec<u8>, ImportError> {
        read_named(
            &self.root,
            "index.json",
            MAX_METADATA_BYTES.min(maximum),
            ImportPhase::SelectManifest,
        )
    }

    pub(crate) fn read_blob(
        &self,
        descriptor: &Descriptor,
        maximum: u64,
        phase: ImportPhase,
    ) -> Result<Vec<u8>, ImportError> {
        let mut file = self.open_blob(descriptor, maximum, phase)?;
        let capacity = usize::try_from(descriptor.size)
            .map_err(|_| ImportError::new(phase, ImportErrorKind::LimitExceeded))?;
        let mut bytes = Vec::with_capacity(capacity);
        let bound = descriptor
            .size
            .checked_add(1)
            .ok_or_else(|| ImportError::new(phase, ImportErrorKind::LimitExceeded))?;
        file.by_ref()
            .take(bound)
            .read_to_end(&mut bytes)
            .map_err(|_| ImportError::new(phase, ImportErrorKind::Io))?;
        if u64::try_from(bytes.len()).ok() != Some(descriptor.size)
            || digest::bytes(&bytes) != descriptor.digest
        {
            return Err(ImportError::new(phase, ImportErrorKind::Integrity));
        }
        Ok(bytes)
    }

    pub(crate) fn open_blob(
        &self,
        descriptor: &Descriptor,
        maximum: u64,
        phase: ImportPhase,
    ) -> Result<File, ImportError> {
        if descriptor.size > maximum {
            return Err(ImportError::new(phase, ImportErrorKind::LimitExceeded));
        }
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = self
            .blobs
            .open_with(digest::hex(&descriptor.digest), &options)
            .map_err(|_| ImportError::new(phase, ImportErrorKind::Io))?;
        let metadata = file
            .metadata()
            .map_err(|_| ImportError::new(phase, ImportErrorKind::Io))?;
        if !metadata.is_file() || metadata.len() != descriptor.size {
            return Err(ImportError::new(phase, ImportErrorKind::Integrity));
        }
        Ok(file)
    }
}

fn read_named(
    dir: &Dir,
    name: &str,
    maximum: u64,
    phase: ImportPhase,
) -> Result<Vec<u8>, ImportError> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = dir
        .open_with(name, &options)
        .map_err(|_| ImportError::new(phase, ImportErrorKind::Io))?;
    let metadata = file
        .metadata()
        .map_err(|_| ImportError::new(phase, ImportErrorKind::Io))?;
    if !metadata.is_file() || metadata.len() > maximum {
        return Err(ImportError::new(phase, ImportErrorKind::LimitExceeded));
    }
    let mut bytes = Vec::new();
    let bound = maximum
        .checked_add(1)
        .ok_or_else(|| ImportError::new(phase, ImportErrorKind::LimitExceeded))?;
    file.by_ref()
        .take(bound)
        .read_to_end(&mut bytes)
        .map_err(|_| ImportError::new(phase, ImportErrorKind::Io))?;
    if u64::try_from(bytes.len())
        .ok()
        .is_none_or(|size| size > maximum)
    {
        return Err(ImportError::new(phase, ImportErrorKind::LimitExceeded));
    }
    Ok(bytes)
}
