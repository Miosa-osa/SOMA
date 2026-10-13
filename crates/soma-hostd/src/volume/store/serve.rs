//! The volume lifecycle the daemon drives, on the directory-backed store.
//!
//! The store is the truth about which volume exists and who owns it; this is what one attach,
//! detach, destroy, or inspection does to it. The offline filesystem work runs here, in the one
//! order that keeps a volume safe: create, then repair, then bind, each with its write-ahead
//! records around it.

use std::path::Path;

use super::{
    AttachOutcome, AttachRequest, DetachOutcome, DetachRequest, DirectoryVolumes, VOLUME_DEVICE,
    VolumeError, VolumeRecordKind, VolumeSidecar, VolumeState, VolumeStore, fs, now_nanos,
};
use crate::VolumeId;
use crate::volume::repair::{CreateError, RepairOutcome};

/// Bytes in one GiB, the unit a volume's class is stated in.
const GIB: u64 = 1 << 30;

impl VolumeStore for DirectoryVolumes {
    fn attach(&self, request: &AttachRequest) -> Result<AttachOutcome, VolumeError> {
        self.check_operation(request.operation, request.fingerprint)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| VolumeError::Ledger("state lock"))?;
        Self::admit(state.get(&request.volume), request)?;
        let existing = state.get(&request.volume);
        let sidecar = VolumeSidecar {
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
            self.transition(VolumeRecordKind::CreateBegin, request.volume)?;
            self.write_sidecar(&VolumeSidecar {
                state: VolumeState::Detached,
                ..sidecar.clone()
            })?;
            // The file and its filesystem are created here, so the repair below runs on a real
            // ext4 filesystem rather than on a path that does not exist yet.
            self.make(request.volume, &file, sidecar.size_class_gib)?;
            self.transition(VolumeRecordKind::CreateEnd, request.volume)?;
        }
        let receipt = self.repairs.repair(request.volume, &file);
        if !receipt.is_usable() {
            return Err(match receipt.outcome {
                RepairOutcome::Refused => {
                    VolumeError::RepairRefused("the volume needs a repair beyond the policy")
                }
                _ => VolumeError::FsckFailed(
                    "the offline repair could not make the volume consistent",
                ),
            });
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
        self.transition(VolumeRecordKind::DestroyBegin, volume)?;
        let directory = self.directory(volume);
        fs::remove_dir_all(&directory).map_err(|_| VolumeError::Ledger("volume unlink"))?;
        self.transition(VolumeRecordKind::DestroyEnd, volume)?;
        state.remove(&volume);
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

impl DirectoryVolumes {
    /// Creates the volume's file and filesystem at the class it was admitted with.
    ///
    /// A refusal here is a volume that does not exist: the file is left as it was found, so a
    /// later attach retries rather than inheriting a half-created volume.
    fn make(&self, volume: VolumeId, file: &Path, class_gib: u32) -> Result<(), VolumeError> {
        let bytes = u64::from(class_gib) * GIB;
        self.repairs
            .create(volume, file, bytes)
            .map_err(|error| match error {
                CreateError::Occupied => VolumeError::Busy("the volume path already holds data"),
                CreateError::Size(_) => VolumeError::Geometry("the volume class has no size"),
                CreateError::Tool(_) => {
                    VolumeError::FsckFailed("the volume filesystem could not be created")
                }
            })
    }
}
