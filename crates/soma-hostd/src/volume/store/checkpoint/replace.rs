//! Durable staging and publication of a replacement for a stopped workspace.

use std::{
    fs::{self, File},
    io::ErrorKind,
    os::unix::fs::MetadataExt as _,
    path::{Path, PathBuf},
};

use sha2::{Digest as _, Sha256};

mod file;
mod intent;
mod interruption;
mod swap;
mod undo;

use file::{inode, lock, open_regular, private_file, sync};
use intent::{INTENT, Intent};
use swap::{publish, require_worker_lock, rollback, verify_stage};

use super::{
    CheckpointVolumeBinding, DirectoryVolumes, IO_FAILURE, Request, Staging, VOLUME_FILE,
    VolumeError, VolumeState, clone_sparse, open_archive,
};
use crate::{
    OperationId,
    volume::{
        AttachOutcome, VOLUME_DEVICE, VolumeRecordKind, WorkspaceRestoreRequest,
        store::{WORKSPACE_RESTORE_PREFIX, serve::owned_volume},
    },
};

pub(super) const RETAINED: &str = "previous.img";
pub(super) const DISPLACED: &str = "displaced.img";
pub(super) const ROLLBACK: &str = "rollback.img";

impl DirectoryVolumes {
    /// Stages a verified checkpoint disk beside its existing, detached workspace.
    ///
    /// The caller must own the stopped Instance's restore lease.
    /// The staging path is derived from the volume and restore operation.
    ///
    /// # Errors
    ///
    /// Refuses an unavailable encrypted store, changed ownership, or an invalid checkpoint.
    pub fn stage_workspace_restore(
        &self,
        request: &WorkspaceRestoreRequest,
    ) -> Result<CheckpointVolumeBinding, VolumeError> {
        self.stage_workspace_from(request, || {
            open_archive(request.checkpoint.source, request.checkpoint.checkpoint)
        })
    }

    fn stage_workspace_from(
        &self,
        request: &WorkspaceRestoreRequest,
        archive: impl FnOnce() -> Result<File, VolumeError>,
    ) -> Result<CheckpointVolumeBinding, VolumeError> {
        let fingerprint = self.restore_fingerprint(request)?;
        let state = self.state.lock().map_err(|_| IO_FAILURE)?;
        let owner = owned_volume(state.values(), request.checkpoint.source)?.clone();
        if owner.generation != request.checkpoint.generation || owner.volume != request.volume {
            return Err(VolumeError::ChangedIntent);
        }
        if owner.state != VolumeState::Detached {
            return Err(VolumeError::Busy("the workspace is attached"));
        }
        let path = restore_path(&self.directory(owner.volume), request.operation);
        let current = self.file(owner.volume);
        let original_file = open_regular(&current, true)?;
        lock(&original_file)?;
        let original = inode(&original_file)?;
        if stage_exists(&path)? {
            let intent = Intent::read(&path, fingerprint)?;
            verify_stage(
                &path,
                intent,
                original,
                owner.last_transition_nanos,
                owner.size_class_gib,
            )?;
        } else {
            self.require_unstaged_workspace(owner.volume)?;
            drop(original_file);
            drop(state);
            let pending = Staging::new(&self.root)?;
            let staged = pending.path.join(VOLUME_FILE);
            clone_sparse(
                archive()?,
                &staged,
                u64::from(owner.size_class_gib) << 30,
                request.checkpoint.digest.as_bytes(),
            )?;
            let intent = Intent {
                fingerprint,
                original,
                replacement: inode(&private_file(&staged, false)?)?,
                transition: owner.last_transition_nanos,
            };
            pending.write(INTENT, &intent.encode())?;
            let state = self.state.lock().map_err(|_| IO_FAILURE)?;
            if owned_volume(state.values(), request.checkpoint.source)? != &owner {
                return Err(VolumeError::ChangedIntent);
            }
            let current = open_regular(&current, true)?;
            lock(&current)?;
            if inode(&current)? != original {
                return Err(VolumeError::ChangedIntent);
            }
            if stage_exists(&path)? {
                let existing = Intent::read(&path, fingerprint)?;
                verify_stage(
                    &path,
                    existing,
                    original,
                    owner.last_transition_nanos,
                    owner.size_class_gib,
                )?;
            } else {
                self.require_unstaged_workspace(owner.volume)?;
                pending.publish(&path)?;
                sync(&self.directory(owner.volume))?;
            }
        }
        Ok(CheckpointVolumeBinding {
            volume: owner.volume,
            size_class_gib: owner.size_class_gib,
            path: path.join(VOLUME_FILE),
        })
    }

    /// Publishes a staged workspace after an authenticated worker holds its disk lock.
    ///
    /// The previous backing stays linked under this restore operation.
    /// The caller holds the same Instance restore lease used for staging.
    ///
    /// # Errors
    ///
    /// Refuses a changed intent, a missing worker lock, or a workspace modified since staging.
    pub fn publish_workspace_restore(
        &self,
        request: &WorkspaceRestoreRequest,
    ) -> Result<(), VolumeError> {
        self.workspace_publication(request, false).map(|_| ())
    }

    pub(in crate::volume::store) fn activate_workspace_restore(
        &self,
        request: &WorkspaceRestoreRequest,
    ) -> Result<AttachOutcome, VolumeError> {
        self.workspace_publication(request, true)
    }

    fn workspace_publication(
        &self,
        request: &WorkspaceRestoreRequest,
        activate: bool,
    ) -> Result<AttachOutcome, VolumeError> {
        let fingerprint = self.restore_fingerprint(request)?;
        let mut state = self.state.lock().map_err(|_| IO_FAILURE)?;
        let owner = owned_volume(state.values(), request.checkpoint.source)?.clone();
        if owner.generation != request.checkpoint.generation || owner.volume != request.volume {
            return Err(VolumeError::ChangedIntent);
        }
        let directory = self.directory(owner.volume);
        let path = restore_path(&directory, request.operation);
        if !stage_exists(&path)? {
            return Err(VolumeError::Missing(
                "the workspace restore has not been staged",
            ));
        }
        let intent = Intent::read(&path, fingerprint)?;
        let backing = self.file(owner.volume);
        let transition = if activate {
            Some(interruption::arm(self, &path)?)
        } else {
            None
        };
        publish(
            &directory,
            &path,
            &backing,
            intent,
            owner.state,
            owner.last_transition_nanos,
        )?;
        if let Some(transition) = transition {
            let worker_disk = open_regular(&backing, true)?;
            require_worker_lock(&worker_disk)?;
            if owner.state == VolumeState::Attached && owner.last_transition_nanos != transition {
                return Err(VolumeError::ChangedIntent);
            }
            if owner.state == VolumeState::Detached {
                let mut attached = owner.clone();
                attached.state = VolumeState::Attached;
                attached.last_transition_nanos = transition;
                self.transition(VolumeRecordKind::AttachBegin, attached.volume)?;
                self.write_sidecar(&attached)?;
                self.transition(VolumeRecordKind::AttachEnd, attached.volume)?;
                state.insert(attached.volume, attached);
            }
        }
        Ok(AttachOutcome {
            volume: request.volume,
            state: if activate {
                VolumeState::Attached
            } else {
                owner.state
            },
            device: VOLUME_DEVICE,
            repair: None,
        })
    }

    fn restore_fingerprint(
        &self,
        request: &WorkspaceRestoreRequest,
    ) -> Result<[u8; 32], VolumeError> {
        if !self.encrypted || request.checkpoint.source != request.checkpoint.destination {
            return Err(VolumeError::Encryption(
                "workspace replacement requires the same Instance and encrypted storage",
            ));
        }
        let mut hash = Sha256::new();
        hash.update(b"soma.workspace.checkpoint.replace.v1");
        hash.update(request.operation.as_bytes());
        hash.update(request.volume.as_bytes());
        hash.update(Request::VolumeCloneCheckpoint(request.checkpoint).encode());
        Ok(hash.finalize().into())
    }
}

fn restore_path(volume: &Path, operation: OperationId) -> PathBuf {
    volume.join(format!("{WORKSPACE_RESTORE_PREFIX}{}", operation.hex()))
}

fn stage_exists(path: &Path) -> Result<bool, VolumeError> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && meta.mode().trailing_zeros() >= 6 => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        _ => Err(IO_FAILURE),
    }
}

#[cfg(test)]
mod tests;
