//! Descriptor staging and durable publication for resident checkpoints.

use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::{ErrorKind, Read as _, Write as _},
    os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _},
    path::{Path, PathBuf},
};

use soma::{GenerationId, InstanceId, OperationId};
use soma_kvm::snapshot::{
    Digest,
    device_state::{DeviceSpecific, DeviceState},
    manifest::CheckpointManifest,
    section::SectionRole,
};
use soma_vmm::sandbox::{CheckpointJob, CheckpointSource};

use crate::{CheckpointFailure, CheckpointReceipt};

use super::{
    held::Held,
    identity::{generation_bytes, instance_bytes},
};

pub(super) const CHECKPOINT_CEILING: std::time::Duration = std::time::Duration::from_mins(15);
const ROOT_VARIABLE: &str = "SOMA_CHECKPOINT_DIR";
const METADATA_LIMIT: u64 = 8 * 1024 * 1024;
const STATE: &str = "state.somackp";
const MEMORY: &str = "memory.raw";
const OVERLAY: &str = "overlay.raw";
const RECEIPT: &str = "receipt.json";

pub(super) fn perform(
    held: &mut Held,
    instance: &InstanceId,
    generation: &GenerationId,
    operation: &OperationId,
    source: CheckpointSource,
    paused: bool,
) -> Result<CheckpointReceipt, CheckpointFailure> {
    let root = configured_root()?;
    let published = root.join(format!("{}.{}", instance.as_str(), operation.as_str()));
    if let Some(receipt) = read_published(&published, instance, generation, operation)? {
        sync_directory(&root)?;
        return Ok(receipt);
    }
    if paused {
        return Err(CheckpointFailure::CaptureRefused);
    }
    let stage = Stage::create(&root)?;
    let job = CheckpointJob {
        source,
        state: stage.open(STATE)?,
        memory: stage.open(MEMORY)?,
        overlay: source
            .has_overlay
            .then(|| stage.open(OVERLAY))
            .transpose()?,
    };
    let reported = held.checkpoint(job)?;
    let receipt = read_staged(&stage.path, instance, generation, operation, Some(reported))?;
    stage.publish(&root, &published, &receipt)?;
    Ok(receipt)
}

fn configured_root() -> Result<PathBuf, CheckpointFailure> {
    let path = std::env::var_os(ROOT_VARIABLE).ok_or(CheckpointFailure::Unsupported)?;
    let path = PathBuf::from(path);
    let directory = File::open(&path).map_err(|_| CheckpointFailure::Storage)?;
    let meta = directory
        .metadata()
        .map_err(|_| CheckpointFailure::Storage)?;
    if !path.is_absolute() || !meta.is_dir() || meta.mode() & 0o077 != 0 {
        return Err(CheckpointFailure::Storage);
    }
    Ok(path)
}

fn read_staged(
    path: &Path,
    instance: &InstanceId,
    generation: &GenerationId,
    operation: &OperationId,
    reported: Option<Digest>,
) -> Result<CheckpointReceipt, CheckpointFailure> {
    let bytes = read_bounded(&path.join(STATE))?;
    let decoded = CheckpointManifest::decode(&bytes).map_err(|_| CheckpointFailure::Storage)?;
    let state_digest = Digest::of(&bytes);
    if reported.is_some_and(|digest| state_digest != digest)
        || decoded.instance_id()
            != &instance_bytes(instance).map_err(|_| CheckpointFailure::Storage)?
        || decoded.header().candidate_id.as_bytes()
            != &generation_bytes(generation).map_err(|_| CheckpointFailure::Storage)?
    {
        return Err(CheckpointFailure::Storage);
    }
    let memory = decoded.header().memory;
    if file_len(&path.join(MEMORY))? != Some(memory.size()) {
        return Err(CheckpointFailure::Storage);
    }
    let overlay = decoded
        .sections()
        .iter()
        .find(|section| section.role() == SectionRole::Device1)
        .map(|section| {
            let slot = section
                .role()
                .device_slot()
                .ok_or(CheckpointFailure::Storage)?;
            let state = DeviceState::decode_for_slot(slot, section.payload())
                .map_err(|_| CheckpointFailure::Storage)?;
            let DeviceSpecific::Block(block) = state.specific() else {
                return Err(CheckpointFailure::Storage);
            };
            let length = block
                .capacity_sectors
                .checked_mul(512)
                .ok_or(CheckpointFailure::Storage)?;
            Ok((length, format!("sha256:{}", block.image_digest)))
        })
        .transpose()?;
    if file_len(&path.join(OVERLAY))? != overlay.as_ref().map(|(length, _)| *length) {
        return Err(CheckpointFailure::Storage);
    }
    let (overlay_bytes, overlay_digest) =
        overlay.map_or((0, None), |(length, digest)| (length, Some(digest)));
    Ok(CheckpointReceipt {
        instance_id: instance.clone(),
        checkpoint_id: operation.clone(),
        generation_id: generation.clone(),
        state_digest: format!("sha256:{state_digest}"),
        memory_digest: format!("sha256:{}", memory.digest()),
        memory_bytes: memory.size(),
        overlay_bytes,
        overlay_digest,
    })
}

fn read_published(
    path: &Path,
    instance: &InstanceId,
    generation: &GenerationId,
    operation: &OperationId,
) -> Result<Option<CheckpointReceipt>, CheckpointFailure> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Ok(meta) if meta.is_dir() => {}
        _ => return Err(CheckpointFailure::Storage),
    }
    let receipt: CheckpointReceipt = serde_json::from_slice(&read_bounded(&path.join(RECEIPT))?)
        .map_err(|_| CheckpointFailure::Storage)?;
    if &receipt.instance_id != instance
        || &receipt.generation_id != generation
        || &receipt.checkpoint_id != operation
    {
        return Err(CheckpointFailure::Storage);
    }
    let verified = read_staged(path, instance, generation, operation, None)?;
    if verified != receipt {
        return Err(CheckpointFailure::Storage);
    }
    Ok(Some(receipt))
}

pub(super) struct RestoreArtifacts {
    pub(super) state: File,
    pub(super) memory: File,
    pub(super) overlay: Option<File>,
    pub(super) digest: Digest,
}

/// Opens the files named by a previously published checkpoint receipt.
pub(super) fn restore_artifacts(
    instance: &InstanceId,
    generation: &GenerationId,
    checkpoint: &OperationId,
) -> Result<RestoreArtifacts, CheckpointFailure> {
    let path = configured_root()?.join(format!("{}.{}", instance.as_str(), checkpoint.as_str()));
    let receipt = read_published(&path, instance, generation, checkpoint)?
        .ok_or(CheckpointFailure::NotFound)?;
    let parsed = GenerationId::new(receipt.state_digest).map_err(|_| CheckpointFailure::Storage)?;
    let digest = generation_bytes(&parsed).map_err(|_| CheckpointFailure::Storage)?;
    Ok(RestoreArtifacts {
        state: open_private(&path.join(STATE))?,
        memory: open_private(&path.join(MEMORY))?,
        overlay: receipt
            .overlay_digest
            .is_some()
            .then(|| open_private(&path.join(OVERLAY)))
            .transpose()?,
        digest: Digest::from_bytes(digest),
    })
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, CheckpointFailure> {
    let mut bytes = Vec::new();
    let file = open_private(path)?;
    file.take(METADATA_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CheckpointFailure::Storage)?;
    if bytes.len() as u64 > METADATA_LIMIT {
        return Err(CheckpointFailure::Storage);
    }
    Ok(bytes)
}

fn open_private(path: &Path) -> Result<File, CheckpointFailure> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| CheckpointFailure::Storage)?;
    let meta = file.metadata().map_err(|_| CheckpointFailure::Storage)?;
    if !meta.is_file() || meta.mode() & 0o077 != 0 {
        return Err(CheckpointFailure::Storage);
    }
    Ok(file)
}

fn file_len(path: &Path) -> Result<Option<u64>, CheckpointFailure> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.is_file() || meta.mode() & 0o077 != 0 {
                return Err(CheckpointFailure::Storage);
            }
            Ok(Some(meta.len()))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(_) => Err(CheckpointFailure::Storage),
    }
}

fn sync_directory(path: &Path) -> Result<(), CheckpointFailure> {
    File::open(path)
        .and_then(|dir| dir.sync_all())
        .map_err(|_| CheckpointFailure::Storage)
}

struct Stage {
    path: PathBuf,
}

impl Stage {
    fn create(root: &Path) -> Result<Self, CheckpointFailure> {
        let suffix = u128::from_be_bytes(super::identity::fresh16());
        let path = root.join(format!(".staging-{suffix:032x}"));
        DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|_| CheckpointFailure::Storage)?;
        Ok(Self { path })
    }

    fn open(&self, name: &str) -> Result<File, CheckpointFailure> {
        OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(self.path.join(name))
            .map_err(|_| CheckpointFailure::Storage)
    }

    fn publish(
        self,
        root: &Path,
        path: &Path,
        receipt: &CheckpointReceipt,
    ) -> Result<(), CheckpointFailure> {
        let bytes = serde_json::to_vec(receipt).map_err(|_| CheckpointFailure::Storage)?;
        let mut file = self.open(RECEIPT)?;
        file.write_all(&bytes)
            .map_err(|_| CheckpointFailure::Storage)?;
        file.sync_all().map_err(|_| CheckpointFailure::Storage)?;
        sync_directory(&self.path)?;
        rename_new(&self.path, path)?;
        sync_directory(root)
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        for name in [RECEIPT, STATE, MEMORY, OVERLAY] {
            let _ignored = fs::remove_file(self.path.join(name));
        }
        let _ignored = fs::remove_dir(&self.path);
    }
}

fn rename_new(source: &Path, destination: &Path) -> Result<(), CheckpointFailure> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|_| CheckpointFailure::Storage)
}
