//! Stores the workspace ownership checks around a live handoff.

use std::{collections::BTreeMap, fs, io::ErrorKind};

use super::{
    AttachOutcome, AttachRequest, DetachRequest, DirectoryVolumes, VOLUME_DEVICE, VOLUME_FILE,
    VolumeError, VolumeRecordKind, VolumeSidecar, VolumeState, WORKSPACE_RESTORE_PREFIX, now_nanos,
    serve::refusal,
};
use crate::{OperationId, VolumeId, volume::VolumeBinding};

impl DirectoryVolumes {
    /// Rejects another mutation while a staged replacement can still be opened by its worker.
    /// Callers hold the volume store mutex. A published operation has no staged volume file.
    pub(super) fn require_unstaged_workspace(&self, volume: VolumeId) -> Result<(), VolumeError> {
        let unavailable = VolumeError::Ledger("workspace restore staging could not be checked");
        for entry in fs::read_dir(self.directory(volume)).map_err(|_| unavailable.clone())? {
            let entry = entry.map_err(|_| unavailable.clone())?;
            let name = entry.file_name();
            let is_restore = name.to_str().is_some_and(|name| {
                name.strip_prefix(WORKSPACE_RESTORE_PREFIX)
                    .is_some_and(|id| OperationId::from_hex(id).is_ok())
            });
            if !is_restore {
                continue;
            }
            if !entry.file_type().map_err(|_| unavailable.clone())?.is_dir() {
                return Err(unavailable);
            }
            match fs::symlink_metadata(entry.path().join(VOLUME_FILE)) {
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Ok(_) => return Err(VolumeError::Busy("a workspace restore is staged")),
                Err(_) => return Err(unavailable),
            }
        }
        Ok(())
    }

    pub(super) fn stage_workspace(
        &self,
        request: &AttachRequest,
    ) -> Result<VolumeBinding, VolumeError> {
        self.check_operation(request.operation, request.fingerprint)?;
        let state = self.state.lock().map_err(|_| lock_error())?;
        let existing = exact_workspace(&state, request)?;
        self.require_encrypted()?;
        self.require_unstaged_workspace(existing.volume)?;
        let path = self.file(existing.volume);
        let repair = self.repairs.repair(existing.volume, &path);
        if !repair.is_usable() {
            return Err(refusal(repair.outcome));
        }
        Ok(VolumeBinding {
            volume: existing.volume,
            path,
            size_class_gib: existing.size_class_gib,
            repair,
        })
    }

    pub(super) fn publish_workspace(
        &self,
        request: &AttachRequest,
    ) -> Result<AttachOutcome, VolumeError> {
        self.check_operation(request.operation, request.fingerprint)?;
        let mut state = self.state.lock().map_err(|_| lock_error())?;
        let existing = exact_workspace(&state, request)?;
        self.require_encrypted()?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.file(existing.volume))
            .map_err(|_| VolumeError::Ledger("workspace file could not be checked"))?;
        match file.try_lock() {
            Err(fs::TryLockError::WouldBlock) => {}
            _ => return Err(VolumeError::Busy("the worker has no workspace lock")),
        }
        if existing.state != VolumeState::Attached {
            let attached = VolumeSidecar {
                state: VolumeState::Attached,
                last_transition_nanos: now_nanos(),
                ..existing.clone()
            };
            self.transition(VolumeRecordKind::AttachBegin, request.volume)?;
            self.write_sidecar(&attached)?;
            self.transition(VolumeRecordKind::AttachEnd, request.volume)?;
            state.insert(request.volume, attached);
        }
        Ok(AttachOutcome {
            volume: request.volume,
            device: VOLUME_DEVICE,
            state: VolumeState::Attached,
            repair: None,
        })
    }

    pub(super) fn admit_workspace_detach(
        &self,
        request: &DetachRequest,
    ) -> Result<VolumeState, VolumeError> {
        self.check_operation(request.operation, request.fingerprint)?;
        let state = self.state.lock().map_err(|_| lock_error())?;
        let existing = state
            .get(&request.volume)
            .ok_or(VolumeError::Missing("the workspace does not exist"))?;
        if existing.instance != request.instance || existing.state == VolumeState::Deleting {
            return Err(VolumeError::Busy(
                "the Instance does not own this workspace",
            ));
        }
        Ok(existing.state)
    }

    fn require_encrypted(&self) -> Result<(), VolumeError> {
        if self.encrypted {
            Ok(())
        } else {
            Err(VolumeError::Encryption(
                "workspace root is not on the encrypted mount",
            ))
        }
    }
}

fn lock_error() -> VolumeError {
    VolumeError::Ledger("state lock")
}

fn exact_workspace<'a>(
    state: &'a BTreeMap<VolumeId, VolumeSidecar>,
    request: &AttachRequest,
) -> Result<&'a VolumeSidecar, VolumeError> {
    let existing = state
        .get(&request.volume)
        .ok_or(VolumeError::Missing("the workspace does not exist"))?;
    DirectoryVolumes::admit(Some(existing), request)?;
    if existing.size_class_gib != request.size_class_gib
        || existing.generation != request.generation
        || existing.tenant != request.tenant
    {
        return Err(VolumeError::ChangedIntent);
    }
    Ok(existing)
}
