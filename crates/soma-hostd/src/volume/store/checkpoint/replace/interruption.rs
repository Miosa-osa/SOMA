//! Reconcile work whose exact broker launch is still pending after its worker has stopped.

use super::{
    DirectoryVolumes, IO_FAILURE, Intent, RETAINED, Staging, VOLUME_FILE, VolumeError,
    VolumeRecordKind, VolumeState, WorkspaceRestoreRequest, fs, inode, lock, open_regular,
    owned_volume, private_file, restore_path, rollback, stage_exists, sync,
};
use crate::volume::record::now_nanos;
use std::{
    io::{ErrorKind, Read as _},
    path::Path,
};

const ACTIVATION: &str = "attached-revision";

pub(super) fn arm(store: &DirectoryVolumes, path: &Path) -> Result<u64, VolumeError> {
    if let Some(revision) = activation(path)? {
        return Ok(revision);
    }
    let revision = now_nanos();
    let pending = Staging::new(&store.root)?;
    pending.write(ACTIVATION, &revision.to_be_bytes())?;
    fs::rename(pending.path.join(ACTIVATION), path.join(ACTIVATION)).map_err(|_| IO_FAILURE)?;
    sync(path)?;
    Ok(revision)
}

fn activation(path: &Path) -> Result<Option<u64>, VolumeError> {
    let marker = path.join(ACTIVATION);
    match fs::symlink_metadata(&marker) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Ok(meta) if meta.is_file() && meta.len() == 8 => {}
        _ => return Err(IO_FAILURE),
    }
    let mut bytes = [0; 8];
    private_file(&marker, false)?
        .read_exact(&mut bytes)
        .map_err(|_| IO_FAILURE)?;
    Ok(Some(u64::from_be_bytes(bytes)))
}

impl DirectoryVolumes {
    /// Reverts an interrupted restore after the broker confirms teardown under its restore lease.
    ///
    /// # Errors
    /// Refuses mismatched ownership, changed revisions, or a worker still using the disk.
    pub(in crate::volume::store) fn reconcile_workspace_restore(
        &self,
        request: &WorkspaceRestoreRequest,
    ) -> Result<(), VolumeError> {
        let fingerprint = self.restore_fingerprint(request)?;
        let mut state = self.state.lock().map_err(|_| IO_FAILURE)?;
        let mut owner = owned_volume(state.values(), request.checkpoint.source)?.clone();
        if owner.generation != request.checkpoint.generation || owner.volume != request.volume {
            return Err(VolumeError::ChangedIntent);
        }
        let directory = self.directory(owner.volume);
        let path = restore_path(&directory, request.operation);
        if !stage_exists(&path)? {
            return Ok(());
        }
        let intent = Intent::read(&path, fingerprint)?;
        let backing = self.file(owner.volume);
        let original_revision = owner.state == VolumeState::Detached
            && owner.last_transition_nanos == intent.transition;
        if path.join(VOLUME_FILE).exists() {
            let original = open_regular(&backing, true)?;
            lock(&original)?;
            if !original_revision || inode(&original)? != intent.original {
                return Err(VolumeError::ChangedIntent);
            }
            return self.discard_staged_restore(&directory, &path, intent);
        }
        let revision = activation(&path)?.ok_or(VolumeError::ChangedIntent)?;
        let attached_revision =
            owner.state == VolumeState::Attached && owner.last_transition_nanos == revision;
        if !original_revision && !attached_revision {
            return Err(VolumeError::ChangedIntent);
        }
        if inode(&open_regular(&path.join(RETAINED), false)?)? != intent.original {
            return Err(VolumeError::ChangedIntent);
        }
        rollback(&directory, &path, &backing, intent)?;
        if attached_revision {
            owner.state = VolumeState::Detached;
            owner.last_transition_nanos = intent.transition;
            self.transition(VolumeRecordKind::DetachBegin, owner.volume)?;
            self.write_sidecar(&owner)?;
            self.transition(VolumeRecordKind::DetachEnd, owner.volume)?;
            state.insert(owner.volume, owner);
        }
        let history = directory.join(format!(
            ".restore-history-{}-{}",
            request.operation.hex(),
            revision
        ));
        if fs::symlink_metadata(&history).is_ok() {
            return Err(VolumeError::ChangedIntent);
        }
        fs::rename(&path, history).map_err(|_| IO_FAILURE)?;
        sync(&directory)
    }
}
