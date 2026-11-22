//! Opening the checkpoint artifacts previously authorized by the broker.

use std::fs::File;

use soma::{GenerationId, InstanceId, OperationId};
use soma_kvm::snapshot::Digest;

use super::{
    CheckpointFailure, CheckpointVolumeReceipt, MEMORY, OVERLAY, STATE, configured_root,
    generation_bytes, open_private, read_published,
};

pub(in crate::backend::kvm) struct RestoreArtifacts {
    pub(in crate::backend::kvm) state: File,
    pub(in crate::backend::kvm) memory: File,
    pub(in crate::backend::kvm) overlay: Option<File>,
    pub(in crate::backend::kvm) digest: Digest,
    pub(in crate::backend::kvm) volume: Option<CheckpointVolumeReceipt>,
}

/// Opens the files named by a previously published checkpoint receipt.
pub(in crate::backend::kvm) fn restore_artifacts(
    instance: &InstanceId,
    generation: &GenerationId,
    checkpoint: &OperationId,
    independent_workspace: bool,
) -> Result<RestoreArtifacts, CheckpointFailure> {
    let path = configured_root()?.join(format!("{}.{}", instance.as_str(), checkpoint.as_str()));
    let receipt = read_published(&path, instance, generation, checkpoint)?
        .ok_or(CheckpointFailure::NotFound)?;
    if receipt.volume.is_some() && !independent_workspace {
        return Err(CheckpointFailure::Unsupported);
    }
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
        volume: receipt.volume,
    })
}
