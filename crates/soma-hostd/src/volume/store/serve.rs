//! The volume lifecycle the daemon drives, on the directory-backed store.
//!
//! The store is the truth about which volume exists and who owns it; this is what one attach,
//! detach, destroy, or inspection does to it. The offline filesystem work runs here, in the one
//! order that keeps a volume safe: create, then repair, then bind, each with its write-ahead
//! records around it.

pub(super) mod create;
mod owner;
mod takeover;

pub(super) use owner::{owned_volume, refusal};

use super::{
    AttachOutcome, AttachRequest, DetachOutcome, DetachRequest, DirectoryVolumes, VOLUME_DEVICE,
    VolumeError, VolumeRecordKind, VolumeSidecar, VolumeState, VolumeStore, fs, now_nanos,
};
use crate::volume::service::VolumeBinding;
use crate::{InstanceId, VolumeId};

impl VolumeStore for DirectoryVolumes {
    #[cfg(target_os = "linux")]
    fn import_workspace(
        &self,
        request: &crate::volume::WorkspaceImportRequest,
    ) -> Result<crate::volume::CheckpointVolumeBinding, VolumeError> {
        self.install_workspace_backup(request)
    }

    #[cfg(target_os = "linux")]
    fn clone_checkpoint(
        &self,
        request: &crate::volume::CheckpointVolumeRequest,
    ) -> Result<crate::volume::CheckpointVolumeBinding, VolumeError> {
        self.copy_checkpoint(request)
    }

    #[cfg(target_os = "linux")]
    fn recovery_stage(
        &self,
        request: &crate::volume::WorkspaceRestoreRequest,
    ) -> Result<crate::volume::CheckpointVolumeBinding, VolumeError> {
        self.stage_workspace_restore(request)
    }

    #[cfg(target_os = "linux")]
    fn recovery_publish(
        &self,
        request: &crate::volume::WorkspaceRestoreRequest,
    ) -> Result<AttachOutcome, VolumeError> {
        self.activate_workspace_restore(request)
    }

    #[cfg(target_os = "linux")]
    fn recovery_abort(
        &self,
        request: &crate::volume::WorkspaceRestoreRequest,
    ) -> Result<(), VolumeError> {
        self.abort_workspace_restore(request)
    }

    fn stage_live_attach(&self, request: &AttachRequest) -> Result<VolumeBinding, VolumeError> {
        self.stage_workspace(request)
    }

    fn commit_live_attach(&self, request: &AttachRequest) -> Result<AttachOutcome, VolumeError> {
        self.publish_workspace(request)
    }

    fn validate_live_detach(&self, request: &DetachRequest) -> Result<VolumeState, VolumeError> {
        self.admit_workspace_detach(request)
    }

    #[cfg(target_os = "linux")]
    fn recovery_reconcile(
        &self,
        request: &crate::volume::WorkspaceRestoreRequest,
    ) -> Result<(), VolumeError> {
        self.reconcile_workspace_restore(request)
    }

    fn attach(&self, request: &AttachRequest) -> Result<AttachOutcome, VolumeError> {
        self.check_operation(request.operation, request.fingerprint)?;
        // A host whose volume root is not on its encrypted data mount refuses the attach before
        // anything is created, so a tenant's bytes are never written to unencrypted disk.
        if !self.encrypted {
            return Err(VolumeError::Encryption(
                "the volume root is not on this host's encrypted data mount",
            ));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| VolumeError::Ledger("state lock"))?;
        let current = takeover::current(
            state.get(&request.volume),
            request,
            &self.file(request.volume),
        )?;
        Self::admit(current.as_ref(), request)?;
        if state
            .values()
            .any(|sidecar| sidecar.instance == request.instance && sidecar.volume != request.volume)
        {
            return Err(VolumeError::Busy(
                "the Instance already owns another volume",
            ));
        }
        let existing = state.get(&request.volume);
        if existing.is_some() {
            self.require_unstaged_workspace(request.volume)?;
        }
        let mut sidecar = VolumeSidecar {
            volume: request.volume,
            instance: request.instance,
            generation: request.generation,
            tenant: request.tenant.clone(),
            size_class_gib: existing.map_or(request.size_class_gib, |value| value.size_class_gib),
            state: VolumeState::Attached,
            dek_id: existing.map_or([0; 16], |value| value.dek_id),
            created_at_nanos: existing.map_or_else(now_nanos, |value| value.created_at_nanos),
            last_transition_nanos: now_nanos(),
        };
        let file = self.file(request.volume);
        if existing.is_none() {
            self.create_volume(request.volume, &mut sidecar, &file)?;
        }
        let receipt = self.repairs.repair(request.volume, &file);
        if !receipt.is_usable() {
            return Err(refusal(receipt.outcome));
        }
        self.transition(VolumeRecordKind::AttachBegin, request.volume)?;
        self.write_sidecar(&sidecar)?;
        self.transition(VolumeRecordKind::AttachEnd, request.volume)?;
        state.insert(request.volume, sidecar);
        Ok(AttachOutcome {
            volume: request.volume,
            device: VOLUME_DEVICE,
            state: VolumeState::Attached,
            repair: Some(receipt),
        })
    }

    fn binding(&self, instance: InstanceId) -> Result<VolumeBinding, VolumeError> {
        // The same rule as an attach: a host whose root is not on its encrypted data mount never
        // hands a tenant's bytes to a machine.
        if !self.encrypted {
            return Err(VolumeError::Encryption(
                "the volume root is not on this host's encrypted data mount",
            ));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| VolumeError::Ledger("state lock"))?;
        // A start resolves the volume this Instance owns, detached or already attached: a stop
        // leaves it detached, and a start is what reattaches it. A volume a delete is in progress
        // on is not one a machine may be given.
        let sidecar = owned_volume(state.values(), instance)?.clone();
        self.require_unstaged_workspace(sidecar.volume)?;
        let file = self.file(sidecar.volume);
        takeover::require_unlocked(&file)?;
        // The repair runs here rather than in the guest, on a file no machine holds yet: this is
        // the one place a start can check the filesystem without racing a running guest.
        let repair = self.repairs.repair(sidecar.volume, &file);
        if !repair.is_usable() {
            return Err(refusal(repair.outcome));
        }
        // The start reattaches: the sidecar says attached before the machine exists, so a start
        // interrupted between here and a running guest leaves a volume the next start finds and
        // attaches again, rather than one no Instance owns.
        let attached = VolumeSidecar {
            state: VolumeState::Attached,
            last_transition_nanos: now_nanos(),
            ..sidecar.clone()
        };
        self.transition(VolumeRecordKind::AttachBegin, sidecar.volume)?;
        self.write_sidecar(&attached)?;
        self.transition(VolumeRecordKind::AttachEnd, sidecar.volume)?;
        state.insert(sidecar.volume, attached);
        Ok(VolumeBinding {
            volume: sidecar.volume,
            path: file,
            size_class_gib: sidecar.size_class_gib,
            repair,
        })
    }

    fn owned(&self, instance: InstanceId) -> Result<VolumeSidecar, VolumeError> {
        let state = self
            .state
            .lock()
            .map_err(|_| VolumeError::Ledger("state lock"))?;
        owned_volume(state.values(), instance).cloned()
    }

    fn detach(&self, request: &DetachRequest) -> Result<DetachOutcome, VolumeError> {
        self.check_operation(request.operation, request.fingerprint)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| VolumeError::Ledger("state lock"))?;
        let Some(existing) = state.get(&request.volume) else {
            return Err(VolumeError::Missing("no sidecar names this volume"));
        };
        if existing.instance != request.instance {
            return Err(VolumeError::Busy("the volume belongs to another Instance"));
        }
        if existing.state == VolumeState::Deleting {
            return Err(VolumeError::Busy("a delete is in progress"));
        }
        self.require_unstaged_workspace(request.volume)?;
        // This inode is held for the lifetime of a worker. Keep the lock until
        // the detached sidecar and its ledger have both been published.
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.file(request.volume))
            .map_err(|_| VolumeError::Ledger("the volume file could not be opened for detach"))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => {
                return Err(VolumeError::Busy("the volume is still in use"));
            }
            Err(fs::TryLockError::Error(_)) => {
                return Err(VolumeError::Ledger("the volume lock could not be checked"));
            }
        }
        let detached = VolumeSidecar {
            state: VolumeState::Detached,
            last_transition_nanos: now_nanos(),
            ..existing.clone()
        };
        self.transition(VolumeRecordKind::DetachBegin, request.volume)?;
        self.write_sidecar(&detached)?;
        self.transition(VolumeRecordKind::DetachEnd, request.volume)?;
        state.insert(request.volume, detached);
        Ok(DetachOutcome {
            volume: request.volume,
            state: VolumeState::Detached,
        })
    }

    fn destroy(&self, volume: VolumeId) -> Result<(), VolumeError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| VolumeError::Ledger("state lock"))?;
        if !state.contains_key(&volume) {
            // Destroying a volume that is already gone reaches the same terminal state, so a
            // replay after a lost answer succeeds rather than reporting a volume nobody owns.
            return Ok(());
        }
        self.require_unstaged_workspace(volume)?;
        // Do not unlink a disk while a live workspace owns its file description.
        let locked = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.file(volume));
        let _guard = match locked {
            Ok(file) => match file.try_lock() {
                Ok(()) => Some(file),
                Err(fs::TryLockError::WouldBlock) => {
                    return Err(VolumeError::Busy("the volume is still in use"));
                }
                Err(fs::TryLockError::Error(_)) => {
                    return Err(VolumeError::Ledger("the volume lock could not be checked"));
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(VolumeError::Ledger("the volume could not be opened")),
        };
        self.transition(VolumeRecordKind::DestroyBegin, volume)?;
        let directory = self.directory(volume);
        fs::remove_dir_all(&directory).map_err(|_| VolumeError::Ledger("volume unlink"))?;
        // The file and the key go together: a volume whose key is gone cannot be read back, which
        // is the erasure a host can actually prove about its medium.
        self.keys
            .destroy(volume)
            .map_err(|_| VolumeError::Encryption("the volume's data key could not be destroyed"))?;
        self.transition(VolumeRecordKind::DestroyEnd, volume)?;
        state.remove(&volume);
        self.forget(volume);
        Ok(())
    }

    fn inspect(&self, volume: VolumeId) -> Result<VolumeSidecar, VolumeError> {
        let state = self
            .state
            .lock()
            .map_err(|_| VolumeError::Ledger("state lock"))?;
        state
            .get(&volume)
            .cloned()
            .ok_or(VolumeError::Missing("no sidecar names this volume"))
    }
}
