use super::{
    DirectoryVolumes, IO_FAILURE, ORIGIN, SIDECAR_FILE, Staging, VOLUME_FILE, clone_sparse,
    open_archive,
};
use crate::{
    Request, RequestFingerprint, VolumeError, VolumeRecordKind, VolumeSidecar, VolumeState,
    volume::{CheckpointVolumeBinding, MAX_TENANT_BYTES, WorkspaceImportRequest},
};
use sha2::{Digest as _, Sha256};
use std::{fs, fs::File};

impl DirectoryVolumes {
    pub(in crate::volume::store) fn install_workspace_backup(
        &self,
        request: &WorkspaceImportRequest,
    ) -> Result<CheckpointVolumeBinding, VolumeError> {
        self.install_verified_workspace(request, || {
            open_archive(request.source, request.checkpoint)
        })
    }

    fn install_verified_workspace(
        &self,
        request: &WorkspaceImportRequest,
        archive: impl FnOnce() -> Result<File, VolumeError>,
    ) -> Result<CheckpointVolumeBinding, VolumeError> {
        if !self.encrypted {
            return Err(VolumeError::Encryption(
                "workspace import requires an encrypted volume store",
            ));
        }
        let target = &request.destination;
        if target.tenant.is_empty()
            || target.tenant.len() > MAX_TENANT_BYTES
            || target.size_class_gib == 0
            || target.operation == request.checkpoint
        {
            return Err(VolumeError::ChangedIntent);
        }
        let intent: [u8; 32] =
            Sha256::digest(Request::WorkspaceImport(request.clone()).encode()).into();
        let operation = RequestFingerprint::new(intent).map_err(|_| IO_FAILURE)?;
        self.check_operation(target.operation, operation)?;
        let path = self.directory(target.volume);
        if let Some(existing) = self.replayed_workspace(request, &intent)? {
            return Ok(existing);
        }
        let pending = Staging::new(&self.root)?;
        clone_sparse(
            archive()?,
            &pending.path.join(VOLUME_FILE),
            u64::from(target.size_class_gib) << 30,
            request.digest.as_bytes(),
        )?;
        pending.write(ORIGIN, &intent)?;

        let mut state = self.state.lock().map_err(|_| IO_FAILURE)?;
        if let Some(existing) = state.get(&target.volume) {
            return self.confirm_workspace(request, &intent, existing);
        }
        if state
            .values()
            .any(|record| record.instance == target.instance)
            || path.exists()
        {
            return Err(VolumeError::Busy(
                "the destination already owns a workspace",
            ));
        }
        self.transition(VolumeRecordKind::CreateBegin, target.volume)?;
        let key = self.keys.seal(target.volume).map_err(|_| IO_FAILURE)?;
        self.keys
            .record(target.volume, &key)
            .map_err(|_| IO_FAILURE)?;
        let now = super::super::now_nanos();
        let sidecar = VolumeSidecar {
            volume: target.volume,
            instance: target.instance,
            generation: target.generation,
            tenant: target.tenant.clone(),
            size_class_gib: target.size_class_gib,
            state: VolumeState::Detached,
            dek_id: key.dek_id,
            created_at_nanos: now,
            last_transition_nanos: now,
        };
        pending.write(SIDECAR_FILE, &sidecar.encode().map_err(|_| IO_FAILURE)?)?;
        pending.publish(&path)?;
        let binding = self.recovery_binding(&sidecar);
        state.insert(target.volume, sidecar);
        self.finish_copy(target.volume)?;
        Ok(binding)
    }

    fn replayed_workspace(
        &self,
        request: &WorkspaceImportRequest,
        intent: &[u8; 32],
    ) -> Result<Option<CheckpointVolumeBinding>, VolumeError> {
        let state = self.state.lock().map_err(|_| IO_FAILURE)?;
        state
            .get(&request.destination.volume)
            .map(|existing| self.confirm_workspace(request, intent, existing))
            .transpose()
    }

    fn confirm_workspace(
        &self,
        request: &WorkspaceImportRequest,
        intent: &[u8; 32],
        existing: &VolumeSidecar,
    ) -> Result<CheckpointVolumeBinding, VolumeError> {
        let target = &request.destination;
        if existing.state == VolumeState::Deleting
            || existing.instance != target.instance
            || existing.tenant != target.tenant
            || existing.generation != target.generation
            || existing.size_class_gib != target.size_class_gib
            || fs::read(self.directory(target.volume).join(ORIGIN))
                .as_deref()
                .ok()
                != Some(intent.as_slice())
        {
            return Err(VolumeError::ChangedIntent);
        }
        self.finish_copy(target.volume)?;
        Ok(self.recovery_binding(existing))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read as _, Seek as _, Write as _},
        os::unix::fs::OpenOptionsExt as _,
    };

    use super::*;
    use crate::{
        AttachRequest, GenerationId, InstanceId, OperationId, VolumeId, VolumeStore,
        volume::{CleanRepair, keyring::HostKeyring},
    };

    #[test]
    fn verified_cold_import_is_detached_durable_and_cannot_overwrite_later_data() {
        let root = tempfile::tempdir().unwrap();
        let record_root = root.path().join("records");
        let volumes = root.path().join("volumes");
        let open = || {
            DirectoryVolumes::open_uncertified(&volumes, &record_root, Box::new(CleanRepair), true)
                .unwrap()
                .with_keys(Box::new(HostKeyring::at(
                    root.path().join("master"),
                    &record_root,
                )))
        };
        let store = open();
        let archive = root.path().join("archive");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&archive)
            .unwrap();
        file.write_all(b"restored marker").unwrap();
        file.set_len(1 << 30).unwrap();
        drop(file);
        let digest = {
            let mut hash = Sha256::new();
            let mut chunk = vec![0_u8; 1 << 20];
            let mut archive = File::open(&archive).unwrap();
            for _ in 0..1024 {
                archive.read_exact(&mut chunk).unwrap();
                hash.update(&chunk);
            }
            RequestFingerprint::new(hash.finalize().into()).unwrap()
        };

        let target = VolumeId::new([3; 16]).unwrap();
        let request = WorkspaceImportRequest {
            source: InstanceId::new([1; 16]).unwrap(),
            checkpoint: OperationId::new([2; 16]).unwrap(),
            digest,
            destination: AttachRequest {
                operation: OperationId::new([5; 16]).unwrap(),
                instance: InstanceId::new([1; 16]).unwrap(),
                volume: target,
                fingerprint: RequestFingerprint::new([6; 32]).unwrap(),
                size_class_gib: 1,
                generation: GenerationId::new([7; 32]).unwrap(),
                tenant: "tenant-alpha".to_owned(),
            },
        };
        let binding = store
            .install_verified_workspace(&request, || File::open(&archive).map_err(|_| IO_FAILURE))
            .unwrap();
        let sidecar = store.inspect(target).unwrap();
        assert_eq!(sidecar.state, VolumeState::Detached);
        assert_eq!(sidecar.generation, request.destination.generation);
        assert_eq!(sidecar.tenant, request.destination.tenant);
        let mut volume = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&binding.path)
            .unwrap();
        let mut marker = [0; 15];
        volume.read_exact(&mut marker).unwrap();
        assert_eq!(&marker, b"restored marker");
        volume.rewind().unwrap();
        volume.write_all(b"updated marker!").unwrap();
        drop(store);

        let restarted = open();
        restarted
            .install_verified_workspace(&request, || panic!("exact replay must not copy again"))
            .unwrap();
        volume.rewind().unwrap();
        volume.read_exact(&mut marker).unwrap();
        assert_eq!(&marker, b"updated marker!");
        let mut wrong = request.clone();
        wrong.destination.tenant = "tenant-beta".to_owned();
        assert!(matches!(
            restarted.install_verified_workspace(&wrong, || panic!(
                "changed ownership must be refused before copying"
            )),
            Err(VolumeError::ChangedIntent)
        ));
    }
}
