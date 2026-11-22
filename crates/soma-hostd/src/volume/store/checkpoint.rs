//! Install a checkpoint's disk in an independent durable workspace.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::{Path, PathBuf},
};

use sha2::{Digest as _, Sha256};

use super::{DirectoryVolumes, SIDECAR_FILE, VOLUME_FILE};

mod replace;
mod stage;

use crate::{
    Request, VolumeError, VolumeId, VolumeRecord, VolumeRecordKind, VolumeSidecar, VolumeState,
    volume::{CheckpointVolumeBinding, CheckpointVolumeRequest, placement},
};
use stage::Staging;
pub(super) use stage::remove_interrupted_stage;

const ORIGIN: &str = "checkpoint-origin";
const IO_FAILURE: VolumeError = VolumeError::Ledger("checkpoint volume storage");
const BAD_SNAPSHOT: VolumeError =
    VolumeError::Geometry("the saved volume differs from its checkpoint");

impl DirectoryVolumes {
    pub(super) fn copy_checkpoint(
        &self,
        request: &CheckpointVolumeRequest,
    ) -> Result<CheckpointVolumeBinding, VolumeError> {
        if !self.encrypted || request.source == request.destination {
            return Err(VolumeError::Encryption(
                "recovery requires an independent encrypted workspace",
            ));
        }
        let id = recovery_id(request)?;
        let path = self.directory(id);
        let intent: [u8; 32] =
            Sha256::digest(Request::VolumeCloneCheckpoint(*request).encode()).into();
        let state = self.state.lock().map_err(|_| IO_FAILURE)?;
        if let Some(existing) = state.get(&id) {
            if existing.instance != request.destination
                || existing.state != VolumeState::Attached
                || fs::read(path.join(ORIGIN)).as_deref().ok() != Some(intent.as_slice())
            {
                return Err(VolumeError::ChangedIntent);
            }
            self.finish_copy(id)?;
            return Ok(self.recovery_binding(existing));
        }
        if state
            .values()
            .any(|record| record.instance == request.destination)
            || path.exists()
        {
            return Err(VolumeError::Busy(
                "the recovery Instance already owns a workspace",
            ));
        }
        let source = super::serve::owned_volume(state.values(), request.source)?;
        if source.generation != request.generation {
            return Err(VolumeError::ChangedIntent);
        }
        let source = source.clone();
        // Copying a large disk can take minutes. Do not hold the store lock while reading it.
        drop(state);
        let archive = open_archive(request)?;
        let pending = Staging::new(&self.root)?;
        clone_sparse(
            archive,
            &pending.path.join(VOLUME_FILE),
            u64::from(source.size_class_gib) << 30,
            request.digest.as_bytes(),
        )?;
        pending.write(ORIGIN, &intent)?;

        let mut state = self.state.lock().map_err(|_| IO_FAILURE)?;
        if super::serve::owned_volume(state.values(), request.source)? != &source {
            return Err(VolumeError::ChangedIntent);
        }
        if state.contains_key(&id)
            || state
                .values()
                .any(|record| record.instance == request.destination)
            || path.exists()
        {
            return Err(VolumeError::Busy(
                "the recovery Instance already owns a workspace",
            ));
        }
        self.transition(VolumeRecordKind::CreateBegin, id)?;
        let key = self.keys.seal(id).map_err(|_| IO_FAILURE)?;
        self.keys.record(id, &key).map_err(|_| IO_FAILURE)?;
        let now = super::now_nanos();
        let restored = VolumeSidecar {
            volume: id,
            instance: request.destination,
            state: VolumeState::Attached,
            dek_id: key.dek_id,
            created_at_nanos: now,
            last_transition_nanos: now,
            ..source
        };
        pending.write(SIDECAR_FILE, &restored.encode().map_err(|_| IO_FAILURE)?)?;
        pending.publish(&path)?;
        let binding = self.recovery_binding(&restored);
        state.insert(id, restored);
        self.finish_copy(id)?;
        Ok(binding)
    }

    fn finish_copy(&self, volume: VolumeId) -> Result<(), VolumeError> {
        File::open(&self.root)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| IO_FAILURE)?;
        let latest = fs::read_dir(self.records.join(volume.hex()))
            .map_err(|_| IO_FAILURE)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| IO_FAILURE)?
            .into_iter()
            .max_by_key(fs::DirEntry::file_name)
            .ok_or(IO_FAILURE)?;
        let bytes = fs::read(latest.path()).map_err(|_| IO_FAILURE)?;
        let record = VolumeRecord::decode(&bytes).ok_or(IO_FAILURE)?;
        if record.volume != volume {
            return Err(IO_FAILURE);
        }
        if record.kind == VolumeRecordKind::CreateBegin {
            self.transition(VolumeRecordKind::CreateEnd, volume)?;
        }
        Ok(())
    }

    fn recovery_binding(&self, restored: &VolumeSidecar) -> CheckpointVolumeBinding {
        CheckpointVolumeBinding {
            volume: restored.volume,
            path: self.file(restored.volume),
            size_class_gib: restored.size_class_gib,
        }
    }
}

fn recovery_id(request: &CheckpointVolumeRequest) -> Result<VolumeId, VolumeError> {
    let mut hash = Sha256::new();
    hash.update(b"soma.workspace.checkpoint.clone.v1");
    hash.update(request.destination.as_bytes());
    let digest = hash.finalize();
    let mut id = [0; 16];
    id.copy_from_slice(&digest[..16]);
    VolumeId::new(id).map_err(|_| IO_FAILURE)
}

fn open_archive(request: &CheckpointVolumeRequest) -> Result<File, VolumeError> {
    let root = PathBuf::from(
        std::env::var_os("SOMA_CHECKPOINT_DIR")
            .ok_or(VolumeError::Missing("checkpoint storage is unavailable"))?,
    );
    let archive = root.join(format!(
        "{}.{}",
        request.source.hex(),
        request.checkpoint.hex()
    ));
    let configured = std::env::var(placement::ENCRYPTED_MOUNT_ENV).ok();
    let mount = placement::encrypted_mount_name(configured.as_deref());
    let private = |path: &Path| {
        fs::symlink_metadata(path)
            .is_ok_and(|meta| meta.is_dir() && meta.mode().trailing_zeros() >= 6)
    };
    if !root.is_absolute()
        || !private(&root)
        || !private(&archive)
        || !placement::root_is_encrypted(&archive, &mount).map_err(|_| IO_FAILURE)?
    {
        return Err(VolumeError::Encryption(
            "checkpoint storage is unavailable or unencrypted",
        ));
    }
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(archive.join("volume.raw"))
        .map_err(|_| VolumeError::Missing("the checkpoint disk is unavailable"))
}

fn clone_sparse(
    mut source: File,
    target: &Path,
    length: u64,
    expected: &[u8; 32],
) -> Result<(), VolumeError> {
    let meta = source.metadata().map_err(|_| IO_FAILURE)?;
    if !meta.is_file() || meta.len() != length || meta.mode() & 0o077 != 0 {
        return Err(BAD_SNAPSHOT);
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(target)
        .map_err(|_| IO_FAILURE)?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    let mut remaining = length;
    while remaining != 0 {
        let n = usize::try_from(remaining.min(buffer.len() as u64)).map_err(|_| IO_FAILURE)?;
        let bytes = &mut buffer[..n];
        source.read_exact(bytes).map_err(|_| IO_FAILURE)?;
        hash.update(&*bytes);
        if bytes.iter().all(|byte| *byte == 0) {
            file.seek(SeekFrom::Current(i64::try_from(n).map_err(|_| IO_FAILURE)?))
                .map_err(|_| IO_FAILURE)?;
        } else {
            file.write_all(bytes).map_err(|_| IO_FAILURE)?;
        }
        remaining -= n as u64;
    }
    if &<[u8; 32]>::from(hash.finalize()) != expected {
        return Err(BAD_SNAPSHOT);
    }
    file.set_len(length)
        .and_then(|()| file.sync_all())
        .map_err(|_| IO_FAILURE)
}

#[cfg(test)]
mod tests;
