//! Opening the checkpoint artifacts previously authorized by the broker.

use std::fs::File;

use soma::{GenerationId, InstanceId, OperationId};
use soma_kvm::snapshot::{Digest, MemoryAttestation};

use super::{
    CheckpointFailure, CheckpointVolumeReceipt, MEMORY, OVERLAY, STATE, configured_root,
    generation_bytes, open_private, read_published,
};

pub(in crate::backend::kvm) struct RestoreArtifacts {
    pub(in crate::backend::kvm) state: File,
    pub(in crate::backend::kvm) memory: File,
    pub(in crate::backend::kvm) overlay: Option<File>,
    pub(in crate::backend::kvm) digest: Digest,
    pub(in crate::backend::kvm) cpu_template: Digest,
    pub(in crate::backend::kvm) volume: Option<CheckpointVolumeReceipt>,
    /// The broker's record of the memory file, when one was written at publication.
    pub(in crate::backend::kvm) memory_attestation: Option<MemoryAttestation>,
}

/// Opens the files named by a previously published checkpoint receipt.
pub(in crate::backend::kvm) fn restore_artifacts(
    instance: &InstanceId,
    generation: &GenerationId,
    checkpoint: &OperationId,
) -> Result<RestoreArtifacts, CheckpointFailure> {
    let path = configured_root()?.join(format!("{}.{}", instance.as_str(), checkpoint.as_str()));
    let published = read_published(&path, instance, generation, checkpoint)?
        .ok_or(CheckpointFailure::NotFound)?;
    let receipt = published.receipt;
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
        cpu_template: published.cpu_template,
        volume: receipt.volume,
        memory_attestation: super::attestation::read(&path),
    })
}
