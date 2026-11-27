//! A private directory a checkpoint is received into from off-host storage, one artifact at a time.
//!
//! The directory sits in the checkpoint root on the encrypted mount, so publishing it is the same
//! atomic rename a local import performs (ADR 0065). Its name starts with `.staging-`, the prefix
//! the retention sweep passes over, so a half-received import is never read as a publication;
//! `limits` gives such stages an age limit and the host a cap on how many and how large they are.
//! Nothing here trusts the bytes: the import verifies every artifact against the caller's receipt
//! before anything becomes visible.
//!
//! One stage is held by one caller at a time: opening it takes an exclusive lock on the directory
//! that receive, commit and abort all hold until they answer.

use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::{ErrorKind, Read, Write as _},
    os::{
        fd::AsFd as _,
        unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _},
    },
    path::{Path, PathBuf},
};

use sha2::{Digest as _, Sha256};
use soma::OperationId;

use super::{
    CheckpointArtifact, CheckpointFailure, CheckpointReceipt, MEMORY, OVERLAY, RECEIPT, STATE,
    VOLUME, configured_root, require_encrypted, sync_directory,
};

mod limits;

const PREFIX: &str = ".staging-import-";
/// Where an artifact is written before it is synced and renamed into place.
const PART: &str = ".part";

/// Why an import stage refused a request, beyond the checkpoint failures every path shares.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportRefusal {
    /// The checkpoint store refused, as every checkpoint path does.
    Checkpoint(CheckpointFailure),
    /// No stage exists for this import.
    Unknown,
    /// Another request holds this import.
    Busy,
    /// The host already holds as many open imports as it accepts.
    TooManyImports,
    /// The artifact does not fit the free space or the staged-bytes cap.
    NoSpace,
}

impl From<CheckpointFailure> for ImportRefusal {
    fn from(failure: CheckpointFailure) -> Self {
        Self::Checkpoint(failure)
    }
}

/// One import being received, named by the caller's import operation, and held by this caller.
#[derive(Debug)]
pub struct ImportStage {
    path: PathBuf,
    // The exclusive lock lives as long as this descriptor.
    _lock: File,
}

impl ImportStage {
    /// Opens the stage for `import`, creating it when it does not exist yet.
    ///
    /// The same operation always names the same directory, so an interrupted transfer can send
    /// the artifacts it has not yet sent, and a different operation can never write into it.
    /// Creating a stage first removes stages older than the age limit, then refuses one more
    /// than the host accepts.
    ///
    /// # Errors
    ///
    /// Refuses a host with no encrypted checkpoint root, a stage that is not a private
    /// directory, a stage another request holds, and a new stage beyond the host's cap.
    pub fn begin(import: &OperationId) -> Result<Self, ImportRefusal> {
        let root = root()?;
        let path = root.join(format!("{PREFIX}{}", import.as_str()));
        if fs::symlink_metadata(&path).is_err() {
            limits::sweep_expired(&root);
            if limits::open_imports(&root) >= limits::MAX_OPEN_IMPORTS {
                return Err(ImportRefusal::TooManyImports);
            }
            match DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => sync_directory(&root)?,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(_) => return Err(CheckpointFailure::Storage.into()),
            }
        }
        Self::lock(path)
    }

    /// Opens the stage for `import` only if it exists, for a commit or an abort.
    ///
    /// # Errors
    ///
    /// Returns `Unknown` for an import with no stage, and the refusals of [`Self::begin`].
    pub fn existing(import: &OperationId) -> Result<Self, ImportRefusal> {
        let path = root()?.join(format!("{PREFIX}{}", import.as_str()));
        match fs::symlink_metadata(&path) {
            Ok(_) => Self::lock(path),
            Err(error) if error.kind() == ErrorKind::NotFound => Err(ImportRefusal::Unknown),
            Err(_) => Err(CheckpointFailure::Storage.into()),
        }
    }

    fn lock(path: PathBuf) -> Result<Self, ImportRefusal> {
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|_| CheckpointFailure::Storage)?;
        let meta = directory
            .metadata()
            .map_err(|_| CheckpointFailure::Storage)?;
        if !meta.is_dir() || meta.mode() & 0o077 != 0 {
            return Err(CheckpointFailure::CaptureRefused.into());
        }
        rustix::fs::flock(
            directory.as_fd(),
            rustix::fs::FlockOperation::NonBlockingLockExclusive,
        )
        .map_err(|_| ImportRefusal::Busy)?;
        Ok(Self {
            path,
            _lock: directory,
        })
    }

    /// The stage directory, which a successful import consumes.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes exactly `length` bytes of one artifact from `source` and returns their SHA-256.
    ///
    /// The bytes go to a temporary name and are renamed into place only once synced, so a
    /// transfer cut short, or a soma-api that died in the middle of one, leaves nothing that
    /// blocks sending the artifact again. A second complete copy is refused, never overwritten.
    ///
    /// # Errors
    ///
    /// Returns `NoSpace` when the artifact does not fit, `CaptureRefused` for an artifact
    /// already received, and `Storage` for a source that ended early or a write that failed.
    pub fn receive(
        &self,
        artifact: CheckpointArtifact,
        length: u64,
        source: &mut impl Read,
    ) -> Result<String, ImportRefusal> {
        let target = self.path.join(artifact.filename());
        if fs::symlink_metadata(&target).is_ok() {
            return Err(CheckpointFailure::CaptureRefused.into());
        }
        limits::require_room(self.path.parent().unwrap_or(&self.path), length)?;
        let part = self.path.join(format!("{}{PART}", artifact.filename()));
        let _ignored = fs::remove_file(&part);
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&part)
            .map_err(|_| CheckpointFailure::Storage)?;
        let written = copy_exact(source, &mut file, length)
            .and_then(|digest| {
                file.sync_all().map_err(|_| CheckpointFailure::Storage)?;
                fs::rename(&part, &target).map_err(|_| CheckpointFailure::Storage)?;
                Ok(digest)
            })
            .inspect_err(|_| {
                let _ignored = fs::remove_file(&part);
            });
        sync_directory(&self.path)?;
        Ok(written?)
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

    /// Removes a stage an import left behind because the capture was already published.
    pub(crate) fn remove_if_unused(&self) -> Result<(), CheckpointFailure> {
        limits::remove_stage(&self.path)
    }

    /// Removes the stage and everything received into it; a stage already consumed is gone.
    ///
    /// # Errors
    ///
    /// Returns `Storage` when the directory could not be removed.
    pub fn abort(self) -> Result<(), CheckpointFailure> {
        limits::remove_stage(&self.path)
    }
}

fn root() -> Result<PathBuf, CheckpointFailure> {
    let root = configured_root()?;
    require_encrypted(&root)?;
    Ok(root)
}

/// Every name a stage may hold: the artifacts, their temporary names, and the receipt.
fn stage_names() -> impl Iterator<Item = String> {
    [STATE, MEMORY, OVERLAY, VOLUME]
        .into_iter()
        .flat_map(|name| [name.to_owned(), format!("{name}{PART}")])
        .chain([RECEIPT.to_owned()])
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
