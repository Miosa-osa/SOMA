//! Read a staged or published checkpoint back and verify it against its receipt.

use super::{
    CheckpointFailure, CheckpointManifest, CheckpointReceipt, CheckpointVolumeReceipt,
    DeviceSpecific, DeviceState, Digest, GenerationId, InstanceId, MEMORY, METADATA_LIMIT, OVERLAY,
    OperationId, RECEIPT, STATE, SectionRole, VOLUME, VerifiedCheckpoint, VolumeId, file_len,
    generation_bytes, instance_bytes, open_private,
};
use std::{
    fs::{self},
    io::{ErrorKind, Read as _},
    path::Path,
};

pub(super) fn read_staged(
    path: &Path,
    instance: &InstanceId,
    generation: &GenerationId,
    operation: &OperationId,
    reported: Option<Digest>,
    volume_id: Option<String>,
) -> Result<VerifiedCheckpoint, CheckpointFailure> {
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
    let overlay = read_block(&decoded, SectionRole::Device1, &path.join(OVERLAY))?;
    let volume = read_block(&decoded, SectionRole::Device6, &path.join(VOLUME))?;
    let (overlay_bytes, overlay_digest) =
        overlay.map_or((0, None), |(length, digest)| (length, Some(digest)));
    Ok(VerifiedCheckpoint {
        receipt: CheckpointReceipt {
            instance_id: instance.clone(),
            checkpoint_id: operation.clone(),
            generation_id: generation.clone(),
            state_digest: format!("sha256:{state_digest}"),
            memory_digest: format!("sha256:{}", memory.digest()),
            memory_bytes: memory.size(),
            overlay_bytes,
            overlay_digest,
            volume: volume.map(|(bytes, digest)| CheckpointVolumeReceipt {
                bytes,
                digest,
                volume_id,
            }),
        },
        cpu_template: decoded.header().cpu_template,
    })
}

pub(super) fn read_block(
    manifest: &CheckpointManifest,
    role: SectionRole,
    path: &Path,
) -> Result<Option<(u64, String)>, CheckpointFailure> {
    let block = manifest
        .sections()
        .iter()
        .find(|section| section.role() == role)
        .map(|section| {
            let slot = role.device_slot().ok_or(CheckpointFailure::Storage)?;
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
    if file_len(path)? != block.as_ref().map(|(length, _)| *length) {
        return Err(CheckpointFailure::Storage);
    }
    Ok(block)
}

pub(super) fn read_published(
    path: &Path,
    instance: &InstanceId,
    generation: &GenerationId,
    operation: &OperationId,
) -> Result<Option<VerifiedCheckpoint>, CheckpointFailure> {
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
    let volume_id = receipt
        .volume
        .as_ref()
        .and_then(|volume| volume.volume_id.clone());
    if let Some(id) = &volume_id {
        let parsed = VolumeId::from_hex(id).map_err(|_| CheckpointFailure::Storage)?;
        if &parsed.hex() != id {
            return Err(CheckpointFailure::Storage);
        }
    }
    let verified = read_staged(path, instance, generation, operation, None, volume_id)?;
    if verified.receipt != receipt {
        return Err(CheckpointFailure::Storage);
    }
    Ok(Some(verified))
}

pub(super) fn read_bounded(path: &Path) -> Result<Vec<u8>, CheckpointFailure> {
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
