//! Descriptor staging and durable publication for resident checkpoints.

mod discard;
mod export;
mod import;
mod import_stage;
mod workspace_import;
pub(crate) use export::open_export;
pub use export::{CheckpointArtifact, CheckpointExport};
pub(crate) use import::{install_import, verify_import};
pub use import_stage::{ImportRefusal, ImportStage};
pub(crate) use workspace_import::import_cold_workspace;
mod attestation;
mod read;
mod restore;
pub(super) mod retention;
pub(super) use attestation::{stamps_hold, vouch_head};
pub(crate) use discard::discard_checkpoint;
use read::{read_bounded, read_published, read_staged};
pub(super) use restore::{RestoreArtifacts, restore_artifacts};

use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::{ErrorKind, Write as _},
    os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _},
    path::{Path, PathBuf},
};

use soma::{GenerationId, InstanceId, OperationId};
use soma_hostd::{
    Reply, Request, VolumeId,
    client::HostClient,
    volume::{
        VolumeState,
        placement::{ENCRYPTED_MOUNT_ENV, encrypted_mount_name, root_is_encrypted},
    },
};
use soma_kvm::snapshot::{
    Digest,
    device_state::{DeviceSpecific, DeviceState},
    manifest::CheckpointManifest,
    section::SectionRole,
};
use soma_vmm::sandbox::{CheckpointJob, CheckpointSource};

use crate::{CheckpointFailure, CheckpointReceipt, checkpoint::CheckpointVolumeReceipt};

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
const VOLUME: &str = "volume.raw";
const RECEIPT: &str = "receipt.json";

struct VerifiedCheckpoint {
    receipt: CheckpointReceipt,
    cpu_template: Digest,
}

pub(super) fn perform(
    held: &mut Held,
    instance: &InstanceId,
    generation: &GenerationId,
    operation: &OperationId,
    source: CheckpointSource,
    paused: bool,
) -> Result<CheckpointReceipt, CheckpointFailure> {
    let root = configured_root()?;
    if source.has_volume {
        let configured = std::env::var(ENCRYPTED_MOUNT_ENV).ok();
        let mount = encrypted_mount_name(configured.as_deref());
        if !root_is_encrypted(&root, &mount).map_err(|_| CheckpointFailure::Storage)? {
            return Err(CheckpointFailure::Unsupported);
        }
    }
    let published = root.join(format!("{}.{}", instance.as_str(), operation.as_str()));
    if let Some(receipt) = read_published(&published, instance, generation, operation)? {
        sync_directory(&root)?;
        return Ok(receipt.receipt);
    }
    if paused {
        return Err(CheckpointFailure::CaptureRefused);
    }
    let volume_id = source
        .has_volume
        .then(|| attached_workspace(instance))
        .transpose()?;
    let stage = Stage::create(&root)?;
    let job = CheckpointJob {
        source,
        state: stage.open(STATE)?,
        memory: stage.open(MEMORY)?,
        overlay: source
            .has_overlay
            .then(|| stage.open(OVERLAY))
            .transpose()?,
        volume: source.has_volume.then(|| stage.open(VOLUME)).transpose()?,
    };
    let reported = held.checkpoint(job)?;
    let receipt = read_staged(
        &stage.path,
        instance,
        generation,
        operation,
        Some(reported),
        volume_id,
    )?
    .receipt;
    stage.publish(&root, &published, &receipt)?;
    // Each sealed artifact is recorded with the digest capture already verified; none is read.
    let sealed = |digest: &String| {
        let digest = GenerationId::new(digest.clone()).ok()?;
        generation_bytes(&digest).ok().map(Digest::from_bytes)
    };
    if let Some(memory) = sealed(&receipt.memory_digest) {
        let overlay = receipt.overlay_digest.as_ref().and_then(sealed);
        attestation::record(&published, memory, overlay);
    }
    Ok(receipt)
}

fn attached_workspace(instance: &InstanceId) -> Result<String, CheckpointFailure> {
    let socket =
        std::env::var_os(super::runtime::SOCKET_VARIABLE).ok_or(CheckpointFailure::Unsupported)?;
    let instance = soma_hostd::InstanceId::new(
        instance_bytes(instance).map_err(|_| CheckpointFailure::Storage)?,
    )
    .map_err(|_| CheckpointFailure::Storage)?;
    match HostClient::connect(&PathBuf::from(socket))
        .and_then(|client| client.call(&Request::VolumeOwned { instance }))
        .map_err(|_| CheckpointFailure::Storage)?
    {
        Reply::VolumeOwned { volume, state } if state == VolumeState::Attached.code() => {
            Ok(volume.hex())
        }
        _ => Err(CheckpointFailure::CaptureRefused),
    }
}

fn require_encrypted(path: &Path) -> Result<(), CheckpointFailure> {
    let configured = std::env::var(ENCRYPTED_MOUNT_ENV).ok();
    let mount = encrypted_mount_name(configured.as_deref());
    if !root_is_encrypted(path, &mount).map_err(|_| CheckpointFailure::Storage)? {
        return Err(CheckpointFailure::Unsupported);
    }
    Ok(())
}

fn published_path(root: &Path, receipt: &CheckpointReceipt) -> PathBuf {
    root.join(format!(
        "{}.{}",
        receipt.instance_id.as_str(),
        receipt.checkpoint_id.as_str()
    ))
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
        for name in [RECEIPT, STATE, MEMORY, OVERLAY, VOLUME] {
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
