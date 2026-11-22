//! Opens only the volume the host's volume store already certified for this launch.

use std::{
    fs::{File, OpenOptions},
    num::NonZeroU64,
    os::unix::fs::OpenOptionsExt as _,
    path::PathBuf,
};

use soma::BackendFailureKind;
use soma_hostd::{
    Reply, Request, VolumeId,
    client::HostClient,
    volume::{CheckpointVolumeRequest, VolumeState, WorkspaceRestoreRequest},
};

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

pub(super) struct RecoveryWorkspace {
    pub(super) attachment: soma::VolumeAttachment,
    staged: Option<(PathBuf, WorkspaceRestoreRequest)>,
}

impl RecoveryWorkspace {
    pub(super) fn publish(&mut self) -> Result<(), BackendFailureKind> {
        let Some((socket, restore)) = &self.staged else {
            return Ok(());
        };
        let request = Request::WorkspaceRestorePublish(*restore);
        let mut reply = HostClient::connect(socket).and_then(|client| client.call(&request));
        if reply.is_err() {
            reply = HostClient::connect(socket).and_then(|client| client.call(&request));
        }
        match reply {
            Ok(Reply::VolumeAttached { volume, state, .. })
                if volume == restore.volume && state == VolumeState::Attached.code() =>
            {
                self.staged = None;
                Ok(())
            }
            _ => Err(BackendFailureKind::Unavailable),
        }
    }
}

impl Drop for RecoveryWorkspace {
    fn drop(&mut self) {
        if let Some((socket, restore)) = &self.staged {
            let _ = HostClient::connect(socket)
                .and_then(|client| client.call(&Request::WorkspaceRestoreAbort(*restore)));
        }
    }
}

pub(super) fn recover(
    launch: &Launching<'_>,
    receipt: &CheckpointVolumeReceipt,
) -> Result<RecoveryWorkspace, BackendFailureKind> {
    let rejected = BackendFailureKind::WorkloadRejected;
    let (source, checkpoint) = launch.checkpoint.ok_or(rejected)?;
    let persistence = &launch.prepared.manifest.persistence;
    if !persistence.enabled || receipt.bytes != u64::from(persistence.size_class_gib) << 30 {
        return Err(rejected);
    }
    let unavailable = BackendFailureKind::Unavailable;
    let digest = soma::GenerationId::new(&receipt.digest).map_err(|_| rejected)?;
    let checkpoint = CheckpointVolumeRequest {
        source: soma_hostd::InstanceId::from_hex(source.as_str()).map_err(|_| rejected)?,
        destination: soma_hostd::InstanceId::new(launch.instance_bytes).map_err(|_| rejected)?,
        checkpoint: soma_hostd::OperationId::from_hex(checkpoint.as_str()).map_err(|_| rejected)?,
        generation: soma_hostd::GenerationId::new(launch.generation_bytes).map_err(|_| rejected)?,
        digest: soma_hostd::RequestFingerprint::new(super::identity::generation_bytes(&digest)?)
            .map_err(|_| rejected)?,
    };
    let restore = if source == launch.instance {
        let volume_id = receipt
            .volume_id
            .as_deref()
            .ok_or(BackendFailureKind::Unsupported)?;
        Some(WorkspaceRestoreRequest {
            checkpoint,
            operation: soma_hostd::OperationId::from_hex(launch.operation.as_str())
                .map_err(|_| rejected)?,
            volume: VolumeId::from_hex(volume_id).map_err(|_| rejected)?,
        })
    } else {
        None
    };
    let request = restore.map_or(
        Request::VolumeCloneCheckpoint(checkpoint),
        Request::WorkspaceRestoreStage,
    );
    let socket =
        PathBuf::from(std::env::var_os(super::runtime::SOCKET_VARIABLE).ok_or(unavailable)?);
    let reply = HostClient::connect(&socket)
        .and_then(|client| client.call(&request))
        .map_err(|_| unavailable)?;
    match reply {
        Reply::VolumeOpened {
            volume,
            size_class_gib,
            path,
            ..
        } if size_class_gib == persistence.size_class_gib
            && restore.is_none_or(|restore| volume == restore.volume) =>
        {
            Ok(RecoveryWorkspace {
                attachment: soma::VolumeAttachment::new(*volume.as_bytes(), PathBuf::from(path)),
                staged: restore.map(|restore| (socket, restore)),
            })
        }
        _ => Err(unavailable),
    }
}
