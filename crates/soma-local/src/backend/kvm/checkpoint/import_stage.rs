//! A private directory a checkpoint is received into from off-host storage, one artifact at a time.
//!
//! The directory sits in the checkpoint root on the encrypted mount, so publishing it is the same
//! atomic rename a local import performs (ADR 0065). Its name starts with `.staging-`, the prefix
//! the retention sweep already passes over, so a half-received import is never read as a
//! publication. Nothing here trusts the bytes: the import verifies every artifact against the
//! caller's receipt before anything becomes visible.

use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::{ErrorKind, Read, Write as _},
    os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _},
    path::{Path, PathBuf},
};

use sha2::{Digest as _, Sha256};
use soma::OperationId;

use super::{
    CheckpointArtifact, CheckpointFailure, CheckpointReceipt, MEMORY, OVERLAY, RECEIPT, STATE,
    VOLUME, configured_root, require_encrypted, sync_directory,
};

/// One import being received, named by the caller's import operation.
#[derive(Debug)]
pub struct ImportStage {
    path: PathBuf,
}

impl ImportStage {
    /// Opens the stage for `import`, creating it on first use.
    ///
    /// The same operation always names the same directory, so an interrupted transfer can send
    /// the artifacts it has not yet sent, and a different operation can never write into it.
    ///
    /// # Errors
    ///
    /// Refuses a host with no checkpoint root, a root off the encrypted mount, and a stage that
    /// is not a private directory.
    pub fn open(import: &OperationId) -> Result<Self, CheckpointFailure> {
        let root = configured_root()?;
        require_encrypted(&root)?;
        let path = root.join(format!(".staging-import-{}", import.as_str()));
        match DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => sync_directory(&root)?,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(_) => return Err(CheckpointFailure::Storage),
        }
        let meta = fs::symlink_metadata(&path).map_err(|_| CheckpointFailure::Storage)?;
        if !meta.is_dir() || meta.mode() & 0o077 != 0 {
            return Err(CheckpointFailure::CaptureRefused);
        }
        Ok(Self { path })
    }

    /// The stage directory, which a successful import consumes.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes exactly `length` bytes of one artifact from `source` and returns their SHA-256.
    ///
    /// An artifact is written once. A transfer that ends short removes what it wrote, so the
    /// caller can send it again; a second complete copy is refused rather than overwritten.
    ///
    /// # Errors
    ///
    /// Returns `CaptureRefused` for an artifact already received, and `Storage` for a source
    /// that ended early or a write that failed.
    pub fn receive(
        &self,
        artifact: CheckpointArtifact,
        length: u64,
        source: &mut impl Read,
    ) -> Result<String, CheckpointFailure> {
        let target = self.path.join(artifact.filename());
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&target)
            .map_err(|error| match error.kind() {
                ErrorKind::AlreadyExists => CheckpointFailure::CaptureRefused,
                _ => CheckpointFailure::Storage,
            })?;
        let written = copy_exact(source, &mut file, length).and_then(|digest| {
            file.sync_all().map_err(|_| CheckpointFailure::Storage)?;
            Ok(digest)
        });
        if written.is_err() {
            let _ignored = fs::remove_file(&target);
        }
        sync_directory(&self.path)?;
        written
    }

    /// Records the receipt the import will be verified against, replacing an earlier one.
    pub(crate) fn record_receipt(
        &self,
        receipt: &CheckpointReceipt,
    ) -> Result<(), CheckpointFailure> {
        let bytes = serde_json::to_vec(receipt).map_err(|_| CheckpointFailure::Storage)?;
        let target = self.path.join(RECEIPT);
        let _ignored = fs::remove_file(&target);
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&target)
            .map_err(|_| CheckpointFailure::Storage)?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| CheckpointFailure::Storage)?;
        sync_directory(&self.path)
    }

    /// Removes the stage and everything received into it.
    ///
    /// # Errors
    ///
    /// Returns `Storage` when the directory could not be removed.
    pub fn abort(self) -> Result<(), CheckpointFailure> {
        for name in [STATE, MEMORY, OVERLAY, VOLUME, RECEIPT] {
            match fs::remove_file(self.path.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(_) => return Err(CheckpointFailure::Storage),
            }
        }
        match fs::remove_dir(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(_) => Err(CheckpointFailure::Storage),
        }
    }
}

fn copy_exact(
    source: &mut impl Read,
    file: &mut File,
    length: u64,
) -> Result<String, CheckpointFailure> {
    let mut remaining = length;
    let mut hash = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    while remaining != 0 {
        let want = usize::try_from(remaining.min(buffer.len() as u64))
            .map_err(|_| CheckpointFailure::Storage)?;
        let read = source
            .read(&mut buffer[..want])
            .map_err(|_| CheckpointFailure::Storage)?;
        if read == 0 {
            return Err(CheckpointFailure::Storage);
        }
        hash.update(&buffer[..read]);
        file.write_all(&buffer[..read])
            .map_err(|_| CheckpointFailure::Storage)?;
        remaining -= read as u64;
    }
    let digest = soma_kvm::snapshot::Digest::from_bytes(hash.finalize().into());
    Ok(format!("sha256:{digest}"))
}

#[cfg(test)]
#[path = "import_stage/tests.rs"]
mod tests;
