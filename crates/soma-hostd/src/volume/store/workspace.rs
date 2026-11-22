//! Stores the workspace ownership checks around a live handoff.

use std::{collections::BTreeMap, fs};

use super::{
    AttachOutcome, AttachRequest, DetachRequest, DirectoryVolumes, VOLUME_DEVICE, VolumeError,
    VolumeRecordKind, VolumeSidecar, VolumeState, now_nanos, serve::refusal,
};
use crate::{VolumeId, volume::VolumeBinding};

impl DirectoryVolumes {
    pub(super) fn stage_workspace(
        &self,
        request: &AttachRequest,
    ) -> Result<VolumeBinding, VolumeError> {
        self.check_operation(request.operation, request.fingerprint)?;
        let state = self.state.lock().map_err(|_| lock_error())?;
        let existing = exact_workspace(&state, request)?;
        self.require_encrypted()?;
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
