use std::{fs, io::ErrorKind, os::unix::fs::MetadataExt as _, path::Path};

use crate::{CheckpointFailure, CheckpointReceipt};

use super::{
    MEMORY, OVERLAY, RECEIPT, STATE, VOLUME, configured_root, export::open_at, open_private,
    published_path, rename_new, require_encrypted, sync_directory,
};

pub(crate) fn install_import(
    stage: &Path,
    expected: &CheckpointReceipt,
) -> Result<(), CheckpointFailure> {
    verify_import(stage, expected)?.install()
}

pub(crate) struct VerifiedImport<'a> {
    root: std::path::PathBuf,
    target: std::path::PathBuf,
    stage: Option<&'a Path>,
}

impl VerifiedImport<'_> {
    pub(crate) fn install(self) -> Result<(), CheckpointFailure> {
        let Some(stage) = self.stage else {
            return sync_directory(&self.root);
        };
        let parent = stage.parent().ok_or(CheckpointFailure::Storage)?;
        rename_new(stage, &self.target)?;
        sync_directory(&self.root)?;
        sync_directory(parent)
    }
}

pub(crate) fn verify_import<'a>(
    stage: &'a Path,
    expected: &CheckpointReceipt,
) -> Result<VerifiedImport<'a>, CheckpointFailure> {
    let root = configured_root()?;
    require_encrypted(&root)?;
    let target = published_path(&root, expected);
    match fs::symlink_metadata(&target) {
        Ok(_) => {
            validate_bundle(&target, expected, true)?;
            return Ok(VerifiedImport {
                root,
                target,
                stage: None,
            });
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(_) => return Err(CheckpointFailure::Storage),
    }
    if !stage.is_absolute() {
        return Err(CheckpointFailure::CaptureRefused);
    }
    require_encrypted(stage)?;
    validate_bundle(stage, expected, false)?;
    Ok(VerifiedImport {
        root,
        target,
        stage: Some(stage),
    })
}

/// Checks one bundle holds exactly the expected artifacts. A checkpoint `published` on this host
/// may also hold the broker's own memory record; an incoming bundle never may, because only the
/// broker that hashed a file can vouch for its stamp.
fn validate_bundle(
    path: &Path,
    expected: &CheckpointReceipt,
    published: bool,
) -> Result<(), CheckpointFailure> {
    let storage = |_| CheckpointFailure::Storage;
    let metadata = fs::symlink_metadata(path).map_err(storage)?;
    if !metadata.is_dir() || metadata.mode() & 0o077 != 0 {
        return Err(CheckpointFailure::CaptureRefused);
    }
    let count =
        3 + usize::from(expected.overlay_digest.is_some()) + usize::from(expected.volume.is_some());
    let mut seen = 0;
    for entry in fs::read_dir(path).map_err(storage)? {
        let name = entry.map_err(storage)?.file_name();
        if published && name.to_str().is_some_and(super::attestation::is_record) {
            continue;
        }
        let allowed = match name.to_str() {
            Some(STATE | MEMORY | RECEIPT) => true,
            Some(OVERLAY) => expected.overlay_digest.is_some(),
            Some(VOLUME) => expected.volume.is_some(),
            _ => false,
        };
        if !allowed {
            return Err(CheckpointFailure::CaptureRefused);
        }
        seen += 1;
    }
    if seen != count {
        return Err(CheckpointFailure::CaptureRefused);
    }

    open_at(path, expected)?.verify_and_sync()?;
    open_private(&path.join(RECEIPT))?
        .sync_all()
        .map_err(storage)?;
    sync_directory(path)
}
