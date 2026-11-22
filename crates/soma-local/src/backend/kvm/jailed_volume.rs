//! Opens only the volume the host's volume store already certified for this launch.

use std::{
    fs::{File, OpenOptions},
    num::NonZeroU64,
    os::unix::fs::OpenOptionsExt as _,
    path::PathBuf,
};

use soma::BackendFailureKind;
use soma_hostd::{Reply, Request, client::HostClient, volume::CheckpointVolumeRequest};

use super::jailed::Launching;
use crate::CheckpointVolumeReceipt;

pub(super) fn open(
    launch: &Launching<'_>,
    recovery_volume: Option<&soma::VolumeAttachment>,
) -> Result<(Option<File>, Option<NonZeroU64>), BackendFailureKind> {
    let persistence = &launch.prepared.manifest.persistence;
    let volume = match (persistence.enabled, launch.volume, recovery_volume) {
        (false, None, None) => return Ok((None, None)),
        (true, Some(volume), None) if launch.checkpoint.is_none() => volume,
        (true, None, Some(volume)) if launch.checkpoint.is_some() => volume,
        _ => return Err(BackendFailureKind::WorkloadRejected),
    };
    let capacity = NonZeroU64::new(u64::from(persistence.size_class_gib) * 1024 * 1024 * 1024)
        .ok_or(BackendFailureKind::WorkloadRejected)?;
    let file = open_locked(&volume.path, capacity)?;
    Ok((Some(file), Some(capacity)))
}

pub(super) fn open_locked(
    path: &std::path::Path,
    capacity: NonZeroU64,
) -> Result<File, BackendFailureKind> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| BackendFailureKind::Unavailable)?;
    let info = file
        .metadata()
        .map_err(|_| BackendFailureKind::Unavailable)?;
    if !info.is_file() || info.len() != capacity.get() {
        return Err(BackendFailureKind::WorkloadRejected);
    }
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            return Err(BackendFailureKind::ResourceConflict);
        }
        Err(std::fs::TryLockError::Error(_)) => {
            return Err(BackendFailureKind::Unavailable);
        }
    }
    Ok(file)
}

pub(super) fn clone_checkpoint(
    launch: &Launching<'_>,
    receipt: &CheckpointVolumeReceipt,
) -> Result<soma::VolumeAttachment, BackendFailureKind> {
    let rejected = BackendFailureKind::WorkloadRejected;
    let (source, checkpoint) = launch.checkpoint.ok_or(rejected)?;
    let persistence = &launch.prepared.manifest.persistence;
    if source == launch.instance
        || !persistence.enabled
        || receipt.bytes != u64::from(persistence.size_class_gib) << 30
    {
        return Err(rejected);
    }
    let unavailable = BackendFailureKind::Unavailable;
    let digest = soma::GenerationId::new(&receipt.digest).map_err(|_| rejected)?;
    let request = Request::VolumeCloneCheckpoint(CheckpointVolumeRequest {
        source: soma_hostd::InstanceId::from_hex(source.as_str()).map_err(|_| rejected)?,
        destination: soma_hostd::InstanceId::new(launch.instance_bytes).map_err(|_| rejected)?,
        checkpoint: soma_hostd::OperationId::from_hex(checkpoint.as_str()).map_err(|_| rejected)?,
        generation: soma_hostd::GenerationId::new(launch.generation_bytes).map_err(|_| rejected)?,
        digest: soma_hostd::RequestFingerprint::new(super::identity::generation_bytes(&digest)?)
            .map_err(|_| rejected)?,
    });
    let socket = std::env::var_os(super::runtime::SOCKET_VARIABLE).ok_or(unavailable)?;
    let reply = HostClient::connect(&PathBuf::from(socket))
        .and_then(|client| client.call(&request))
        .map_err(|_| unavailable)?;
    match reply {
        Reply::VolumeOpened {
            volume,
            size_class_gib,
            path,
            ..
        } if size_class_gib == persistence.size_class_gib => Ok(soma::VolumeAttachment::new(
            *volume.as_bytes(),
            PathBuf::from(path),
        )),
        _ => Err(unavailable),
    }
}
