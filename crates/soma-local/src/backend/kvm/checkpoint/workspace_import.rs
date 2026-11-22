use std::path::Path;

use soma_hostd::{
    AttachRequest, Reply, Request, VolumeId, client::HostClient, volume::WorkspaceImportRequest,
};

use crate::{CheckpointFailure, CheckpointReceipt, ColdWorkspaceTarget};

pub(crate) fn import_cold_workspace(
    stage: &Path,
    expected: &CheckpointReceipt,
    target: &ColdWorkspaceTarget,
) -> Result<(), CheckpointFailure> {
    let rejected = |_| CheckpointFailure::CaptureRefused;
    let archive = expected
        .volume
        .as_ref()
        .ok_or(CheckpointFailure::NotFound)?;
    if archive.bytes % (1 << 30) != 0 {
        return Err(CheckpointFailure::CaptureRefused);
    }
    let size_class_gib = u32::try_from(archive.bytes >> 30).map_err(rejected)?;
    let digest =
        soma::GenerationId::new(&archive.digest).map_err(|_| CheckpointFailure::CaptureRefused)?;
    let bytes = super::generation_bytes(&digest).map_err(|_| CheckpointFailure::CaptureRefused)?;
    let target_volume =
        VolumeId::from_hex(&target.volume).map_err(|_| CheckpointFailure::CaptureRefused)?;
    let target_generation = super::generation_bytes(&target.generation)
        .map_err(|_| CheckpointFailure::CaptureRefused)?;
    let request = Request::WorkspaceImport(WorkspaceImportRequest {
        source: soma_hostd::InstanceId::from_hex(expected.instance_id.as_str())
            .map_err(|_| CheckpointFailure::CaptureRefused)?,
        checkpoint: soma_hostd::OperationId::from_hex(expected.checkpoint_id.as_str())
            .map_err(|_| CheckpointFailure::CaptureRefused)?,
        digest: soma_hostd::RequestFingerprint::new(bytes)
            .map_err(|_| CheckpointFailure::CaptureRefused)?,
        destination: AttachRequest {
            instance: soma_hostd::InstanceId::from_hex(target.instance.as_str())
                .map_err(|_| CheckpointFailure::CaptureRefused)?,
            operation: soma_hostd::OperationId::from_hex(target.operation.as_str())
                .map_err(|_| CheckpointFailure::CaptureRefused)?,
            generation: soma_hostd::GenerationId::new(target_generation)
                .map_err(|_| CheckpointFailure::CaptureRefused)?,
            tenant: target.tenant.clone(),
            volume: target_volume,
            size_class_gib,
            fingerprint: soma_hostd::RequestFingerprint::new(bytes)
                .map_err(|_| CheckpointFailure::CaptureRefused)?,
        },
    });
    let socket = std::env::var_os(super::super::runtime::SOCKET_VARIABLE)
        .ok_or(CheckpointFailure::Unsupported)?;
    super::verify_import(stage, expected)?.install()?;
    match HostClient::connect(std::path::Path::new(&socket))
        .and_then(|client| client.call(&request))
    {
        Ok(Reply::VolumeOpened {
            volume,
            size_class_gib: class,
            ..
        }) if volume == target_volume && class == size_class_gib => Ok(()),
        Ok(Reply::Failed(_)) => Err(CheckpointFailure::CaptureRefused),
        _ => Err(CheckpointFailure::Unconfirmed),
    }
}
