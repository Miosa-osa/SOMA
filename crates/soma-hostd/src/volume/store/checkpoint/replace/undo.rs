//! Abandon a staged workspace restore, or roll a published one back to the preceding backing.

use std::{fs, path::Path};

use super::{
    DirectoryVolumes, IO_FAILURE, Intent, Staging, VOLUME_FILE, VolumeError, VolumeState,
    WorkspaceRestoreRequest, inode, lock, open_regular, owned_volume, private_file, restore_path,
    rollback, stage_exists, sync,
};

impl DirectoryVolumes {
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

    pub(super) fn discard_staged_restore(
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
}
