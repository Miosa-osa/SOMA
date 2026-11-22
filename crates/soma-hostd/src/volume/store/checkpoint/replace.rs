//! Durable staging and publication of a replacement for a stopped workspace.

use std::{
    fs::{self, File},
    io::{ErrorKind, Read as _},
    os::unix::fs::MetadataExt as _,
    path::{Path, PathBuf},
};

use sha2::{Digest as _, Sha256};

mod file;
mod interruption;

use file::{inode, lock, open_regular, private_file, sync};

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

const INTENT: &str = "restore-intent";
const RETAINED: &str = "previous.img";
const DISPLACED: &str = "displaced.img";
const ROLLBACK: &str = "rollback.img";
const INTENT_BYTES: usize = 56;

#[derive(Clone, Copy)]
struct Intent {
    fingerprint: [u8; 32],
    original: u64,
    replacement: u64,
    transition: u64,
}

impl Intent {
    fn read(path: &Path, fingerprint: [u8; 32]) -> Result<Self, VolumeError> {
        let mut bytes = [0; INTENT_BYTES];
        let mut file = private_file(&path.join(INTENT), false)?;
        file.read_exact(&mut bytes).map_err(|_| IO_FAILURE)?;
        if file.metadata().map_err(|_| IO_FAILURE)?.len() != INTENT_BYTES as u64 {
            return Err(IO_FAILURE);
        }
        if bytes[..32] != fingerprint {
            return Err(VolumeError::ChangedIntent);
        }
        let word = |offset| {
            let mut bytes_at = [0; 8];
            bytes_at.copy_from_slice(&bytes[offset..offset + 8]);
            u64::from_be_bytes(bytes_at)
        };
        Ok(Self {
            fingerprint,
            original: word(32),
            replacement: word(40),
            transition: word(48),
        })
    }

    fn encode(self) -> [u8; INTENT_BYTES] {
        let mut bytes = [0; INTENT_BYTES];
        bytes[..32].copy_from_slice(&self.fingerprint);
        bytes[32..40].copy_from_slice(&self.original.to_be_bytes());
        bytes[40..48].copy_from_slice(&self.replacement.to_be_bytes());
        bytes[48..].copy_from_slice(&self.transition.to_be_bytes());
        bytes
    }
}

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
        self.stage_workspace_from(request, || open_archive(&request.checkpoint))
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

    /// Discards an uncommitted replacement after the failed worker released its backing.
    ///
    /// A committed replacement and the prior version are left available for explicit rollback.
    ///
    /// # Errors
    ///
    /// Refuses a changed restore intent, an active stage, or an unavailable encrypted store.
    pub fn abort_workspace_restore(
        &self,
        request: &WorkspaceRestoreRequest,
    ) -> Result<(), VolumeError> {
        let fingerprint = self.restore_fingerprint(request)?;
        let state = self.state.lock().map_err(|_| IO_FAILURE)?;
        let owner = owned_volume(state.values(), request.checkpoint.source)?;
        if owner.generation != request.checkpoint.generation || owner.volume != request.volume {
            return Err(VolumeError::ChangedIntent);
        }
        let directory = self.directory(owner.volume);
        let path = restore_path(&directory, request.operation);
        if !stage_exists(&path)? {
            return Ok(());
        }
        let intent = Intent::read(&path, fingerprint)?;
        let backing = open_regular(&self.file(owner.volume), false)?;
        if inode(&backing)? == intent.replacement {
            return Ok(());
        }
        if inode(&backing)? != intent.original {
            return Err(VolumeError::ChangedIntent);
        }
        self.discard_staged_restore(&directory, &path, intent)
    }

    fn discard_staged_restore(
        &self,
        directory: &Path,
        path: &Path,
        intent: Intent,
    ) -> Result<(), VolumeError> {
        let staged = private_file(&path.join(VOLUME_FILE), true)?;
        if inode(&staged)? != intent.replacement {
            return Err(VolumeError::ChangedIntent);
        }
        lock(&staged)?;
        let retired = Staging::new(&self.root)?;
        fs::rename(path, retired.path.join("retired-restore")).map_err(|_| IO_FAILURE)?;
        sync(&retired.path)?;
        sync(directory)
    }

    /// Restores the preceding backing after the replacement's worker has stopped.
    ///
    /// Retains a link to the displaced replacement so a rollback does not erase later writes.
    /// The caller must hold a stopped Instance's separately authorized rollback lease.
    ///
    /// # Errors
    ///
    /// Refuses an attached workspace or a volume whose restore operation is no longer current.
    pub fn rollback_workspace_restore(
        &self,
        request: &WorkspaceRestoreRequest,
    ) -> Result<(), VolumeError> {
        let fingerprint = self.restore_fingerprint(request)?;
        let state = self.state.lock().map_err(|_| IO_FAILURE)?;
        let owner = owned_volume(state.values(), request.checkpoint.source)?;
        if owner.generation != request.checkpoint.generation || owner.volume != request.volume {
            return Err(VolumeError::ChangedIntent);
        }
        if owner.state != VolumeState::Detached {
            return Err(VolumeError::Busy("the workspace is attached"));
        }
        let directory = self.directory(owner.volume);
        let path = restore_path(&directory, request.operation);
        if !stage_exists(&path)? {
            return Err(VolumeError::Missing("the workspace restore does not exist"));
        }
        let intent = Intent::read(&path, fingerprint)?;
        rollback(&directory, &path, &self.file(owner.volume), intent)
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

fn verify_stage(
    path: &Path,
    intent: Intent,
    original: u64,
    transition: u64,
    class_gib: u32,
) -> Result<(), VolumeError> {
    if original != intent.original || transition != intent.transition {
        return Err(VolumeError::ChangedIntent);
    }
    let staged = private_file(&path.join(VOLUME_FILE), true)?;
    let meta = staged.metadata().map_err(|_| IO_FAILURE)?;
    if meta.ino() != intent.replacement || meta.len() != u64::from(class_gib) << 30 {
        return Err(VolumeError::ChangedIntent);
    }
    lock(&staged)
}

fn publish(
    directory: &Path,
    path: &Path,
    backing: &Path,
    intent: Intent,
    state: VolumeState,
    transition: u64,
) -> Result<(), VolumeError> {
    let current = open_regular(backing, true)?;
    let current_inode = inode(&current)?;
    let retained = path.join(RETAINED);
    if current_inode == intent.replacement {
        if inode(&open_regular(&retained, false)?)? != intent.original {
            return Err(VolumeError::ChangedIntent);
        }
        sync(directory)?;
        return sync(path);
    }
    if current_inode != intent.original
        || transition != intent.transition
        || state != VolumeState::Detached
    {
        return Err(VolumeError::ChangedIntent);
    }
    lock(&current)?;
    let staged_path = path.join(VOLUME_FILE);
    let staged = private_file(&staged_path, true)?;
    if inode(&staged)? != intent.replacement {
        return Err(VolumeError::ChangedIntent);
    }
    require_worker_lock(&staged)?;
    retain_link(backing, &retained, intent.original)?;
    sync(path)?;
    fs::rename(&staged_path, backing).map_err(|_| IO_FAILURE)?;
    sync(directory)?;
    sync(path)
}

fn rollback(
    directory: &Path,
    path: &Path,
    backing: &Path,
    intent: Intent,
) -> Result<(), VolumeError> {
    if path.join(VOLUME_FILE).exists() {
        return Err(VolumeError::Busy(
            "the workspace restore has not been published",
        ));
    }
    let previous_path = path.join(RETAINED);
    let previous = open_regular(&previous_path, true)?;
    if inode(&previous)? != intent.original {
        return Err(VolumeError::ChangedIntent);
    }
    let current = open_regular(backing, true)?;
    lock(&current)?;
    let displaced = path.join(DISPLACED);
    match inode(&current)? {
        current_inode if current_inode == intent.original => {
            if inode(&private_file(&displaced, false)?)? != intent.replacement {
                return Err(VolumeError::ChangedIntent);
            }
            sync(directory)?;
            return sync(path);
        }
        current_inode if current_inode == intent.replacement => {}
        _ => return Err(VolumeError::ChangedIntent),
    }

    lock(&previous)?;
    retain_link(backing, &displaced, intent.replacement)?;
    let restored = path.join(ROLLBACK);
    retain_link(&previous_path, &restored, intent.original)?;
    sync(path)?;
    fs::rename(&restored, backing).map_err(|_| IO_FAILURE)?;
    sync(directory)?;
    sync(path)
}

fn require_worker_lock(staged: &File) -> Result<(), VolumeError> {
    match staged.try_lock() {
        Err(fs::TryLockError::WouldBlock) => Ok(()),
        Ok(()) => Err(VolumeError::Busy(
            "the recovered worker does not own its workspace",
        )),
        Err(fs::TryLockError::Error(_)) => Err(IO_FAILURE),
    }
}

fn retain_link(source: &Path, destination: &Path, expected: u64) -> Result<(), VolumeError> {
    match fs::hard_link(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            if inode(&open_regular(destination, false)?)? != expected {
                return Err(VolumeError::ChangedIntent);
            }
            Ok(())
        }
        Err(_) => Err(IO_FAILURE),
    }
}

#[cfg(test)]
mod tests;
