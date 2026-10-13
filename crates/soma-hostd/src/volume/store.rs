//! The directory-backed volume store: one sidecar beside each file, records under a durable root.
//!
//! The state of the store is the file system, so a restart rebuilds it from the same sidecars the
//! reboot rebuild reads. The write-ahead records are append-only, one create-exclusive checksummed
//! file per sequence number, synced together with their directory before the append returns, which
//! is the protocol the worker ledger already uses.
//! This is the reference implementation of the volume bookkeeping; the offline repair it runs is
//! the [`VolumeRepair`] seam, whose real implementation invokes `e2fsck` on a real detached file.

use std::{
    collections::BTreeMap,
    fs, io,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

use crate::{OperationId, RequestFingerprint, VolumeId};

use super::error::VolumeError;
use super::record::{VolumeRecord, VolumeRecordKind, now_nanos};
use super::repair::{RepairOutcome, RepairReceipt, VolumeRepair};
use super::service::{
    AttachOutcome, AttachRequest, DetachOutcome, DetachRequest, VOLUME_DEVICE, VolumeStore,
};
use super::sidecar::{VolumeSidecar, VolumeState};

/// The sparse ext4 file inside one volume directory.
pub const VOLUME_FILE: &str = "volume.img";
/// The sidecar inside one volume directory.
pub const SIDECAR_FILE: &str = "volume.json";

/// A store whose truth is the file system, with an in-memory index rebuilt from the sidecars.
pub struct DirectoryVolumes {
    root: PathBuf,
    records: PathBuf,
    repairs: Box<dyn VolumeRepair + Send + Sync>,
    state: Mutex<BTreeMap<VolumeId, VolumeSidecar>>,
    operations: Mutex<BTreeMap<OperationId, RequestFingerprint>>,
}

impl DirectoryVolumes {
    /// Opens or creates a store rooted at `root` with its records under `records`.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`io::Error`] when a directory cannot be created or a sidecar
    /// cannot be read.
    pub fn open(
        root: &Path,
        records: &Path,
        repairs: Box<dyn VolumeRepair + Send + Sync>,
    ) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        fs::create_dir_all(records)?;
        let mut state = BTreeMap::new();
        for entry in fs::read_dir(root)? {
            let path = entry?.path().join(SIDECAR_FILE);
            if let Ok(bytes) = fs::read(&path)
                && let Ok(sidecar) = VolumeSidecar::decode(&bytes)
            {
                state.insert(sidecar.volume, sidecar);
            }
        }
        Ok(Self {
            root: root.to_path_buf(),
            records: records.to_path_buf(),
            repairs,
            state: Mutex::new(state),
            operations: Mutex::new(BTreeMap::new()),
        })
    }

    /// The directory holding one volume's file and sidecar.
    #[must_use]
    pub fn directory(&self, volume: VolumeId) -> PathBuf {
        self.root.join(volume.hex())
    }

    /// The volume's backing file.
    #[must_use]
    pub fn file(&self, volume: VolumeId) -> PathBuf {
        self.directory(volume).join(VOLUME_FILE)
    }

    /// Rejects a replay that presents a changed intent under an operation that already ran.
    fn check_operation(
        &self,
        operation: OperationId,
        fingerprint: RequestFingerprint,
    ) -> Result<(), VolumeError> {
        let mut operations = self
            .operations
            .lock()
            .map_err(|_| VolumeError::Ledger("operation lock"))?;
        match operations.get(&operation) {
            Some(seen) if *seen != fingerprint => Err(VolumeError::ChangedIntent),
            Some(_) => Ok(()),
            None => {
                operations.insert(operation, fingerprint);
                Ok(())
            }
        }
    }

    /// Appends one record and returns after its file and directory are synced.
    fn append(&self, record: &VolumeRecord) -> Result<(), VolumeError> {
        let directory = self.records.join(record.volume.hex());
        fs::create_dir_all(&directory).map_err(|_| VolumeError::Ledger("record directory"))?;
        let sequence = fs::read_dir(&directory)
            .map_err(|_| VolumeError::Ledger("record directory"))?
            .count();
        let path = directory.join(format!("{sequence:020}"));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|_| VolumeError::Ledger("record file"))?;
        file.write_all(&record.encode())
            .map_err(|_| VolumeError::Ledger("record write"))?;
        file.sync_all()
            .map_err(|_| VolumeError::Ledger("record sync"))?;
        fs::File::open(&directory)
            .and_then(|handle| handle.sync_all())
            .map_err(|_| VolumeError::Ledger("record directory sync"))?;
        Ok(())
    }

    /// Writes the sidecar, which states the volume's durable state before it is acted on.
    fn write_sidecar(&self, sidecar: &VolumeSidecar) -> Result<(), VolumeError> {
        let directory = self.directory(sidecar.volume);
        fs::create_dir_all(&directory).map_err(|_| VolumeError::Ledger("volume directory"))?;
        let bytes = sidecar
            .encode()
            .map_err(|_| VolumeError::Ledger("sidecar"))?;
        fs::write(directory.join(SIDECAR_FILE), bytes)
            .map_err(|_| VolumeError::Ledger("sidecar write"))
    }

    /// Records one transition in the durable write-ahead records.
    fn transition(&self, kind: VolumeRecordKind, volume: VolumeId) -> Result<(), VolumeError> {
        self.append(&VolumeRecord::new(kind, volume))
    }

    /// Refuses a request the current sidecar does not allow.
    fn admit(existing: Option<&VolumeSidecar>, request: &AttachRequest) -> Result<(), VolumeError> {
        let Some(sidecar) = existing else {
            return Ok(());
        };
        match sidecar.state {
            VolumeState::Deleting => Err(VolumeError::Busy("a delete is in progress")),
            VolumeState::Attached | VolumeState::Detached
                if sidecar.instance != request.instance =>
            {
                Err(VolumeError::Busy("the volume belongs to another Instance"))
            }
            VolumeState::Attached | VolumeState::Detached => {
                if request.size_class_gib < sidecar.size_class_gib {
                    Err(VolumeError::Geometry(
                        "the requested class is below the volume's own",
                    ))
                } else {
                    Ok(())
                }
            }
        }
    }
}

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
        if existing.is_none() {
            self.transition(VolumeRecordKind::CreateBegin, request.volume)?;
            self.write_sidecar(&VolumeSidecar {
                state: VolumeState::Detached,
                ..sidecar.clone()
            })?;
            self.transition(VolumeRecordKind::CreateEnd, request.volume)?;
        }
        let file = self.file(request.volume);
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

/// A repair that always reports a clean filesystem, for a machine with no ext4 file to check.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CleanRepair;

impl VolumeRepair for CleanRepair {
    fn repair(&self, volume: VolumeId, _file: &Path) -> RepairReceipt {
        RepairReceipt::new(volume, RepairOutcome::Clean, 0, now_nanos())
    }
}

#[cfg(test)]
mod tests;
