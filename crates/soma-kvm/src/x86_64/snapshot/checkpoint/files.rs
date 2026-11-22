//! Copy and verify checkpoint storage while retaining the broker's private handles.

use std::{fs::File, os::unix::fs::FileExt as _};

use crate::{
    snapshot::{Digest, Hasher, manifest::CheckpointManifest},
    virtio::{BlockDevice, Slot},
};

use super::super::{Artifact, SnapshotError, device};

pub(super) const CHUNK: usize = 1 << 20;

pub(super) fn capture_block(
    artifact: Artifact,
    device: Option<&mut BlockDevice>,
    target: Option<&File>,
) -> Result<Digest, SnapshotError> {
    match (device, target) {
        (None, None) => Ok(Digest::from_bytes([0; 32])),
        (Some(device), Some(file)) => {
            device
                .flush_checkpoint()
                .map_err(|_| SnapshotError::NotQuiescent("could not flush the checkpoint disk"))?;
            copy_image(
                artifact,
                file,
                device.capacity_sectors() * 512,
                |offset, bytes| match device.read_checkpoint(offset, bytes) {
                    Ok(n) if n == bytes.len() => Ok(()),
                    _ => Err(SnapshotError::NotQuiescent(
                        "could not read the checkpoint disk",
                    )),
                },
            )
        }
        _ => Err(SnapshotError::NotQuiescent(
            "checkpoint block device target mismatch",
        )),
    }
}

pub(super) fn block_digest(
    manifest: &CheckpointManifest,
    slot: Slot,
) -> Result<Digest, SnapshotError> {
    use crate::snapshot::device_state::{DeviceSpecific, DeviceState};

    let section = manifest
        .sections()
        .iter()
        .find(|section| Some(section.role()) == device::role(slot))
        .ok_or(SnapshotError::MissingSection("checkpoint block device"))?;
    let state = DeviceState::decode_for_slot(slot.index(), section.payload())?;
    match state.specific() {
        DeviceSpecific::Block(block) => Ok(block.image_digest),
        _ => Err(SnapshotError::DeviceStateNotCanonical(slot)),
    }
}

pub(super) fn copy_image(
    artifact: Artifact,
    file: &File,
    size: u64,
    mut read: impl FnMut(u64, &mut [u8]) -> Result<(), SnapshotError>,
) -> Result<Digest, SnapshotError> {
    let mut buffer = vec![0_u8; CHUNK];
    let mut digest = Hasher::new();
    let mut offset = 0_u64;
    while offset < size {
        let span = usize::try_from(size - offset).unwrap_or(CHUNK).min(CHUNK);
        let bytes = &mut buffer[..span];
        read(offset, bytes)?;
        let end = offset + u64::try_from(span).expect("the chunk length fits a byte offset");
        if bytes.iter().any(|&byte| byte != 0) {
            write_at(artifact, file, bytes, offset)?;
        } else if end == size {
            // The broker supplied an empty file. A write at its last byte preserves a
            // trailing hole and the exact length without another jailed syscall.
            write_at(artifact, file, &[0], end - 1)?;
        }
        digest.update(bytes);
        offset = end;
    }
    Ok(digest.finish())
}

pub(super) fn write_at(
    artifact: Artifact,
    file: &File,
    bytes: &[u8],
    offset: u64,
) -> Result<(), SnapshotError> {
    file.write_all_at(bytes, offset)
        .map_err(|error| SnapshotError::io(artifact, "write checkpoint staging file", &error))
}

pub(super) fn seal(artifact: Artifact, file: &File, expected: Digest) -> Result<(), SnapshotError> {
    file.sync_all()
        .map_err(|error| SnapshotError::io(artifact, "sync checkpoint staging file", &error))?;
    let length = file
        .metadata()
        .map_err(|error| SnapshotError::io(artifact, "size checkpoint staging file", &error))?
        .len();
    let mut buffer = vec![0_u8; CHUNK];
    let mut digest = Hasher::new();
    let mut offset = 0_u64;
    while offset < length {
        let span = usize::try_from(length - offset).unwrap_or(CHUNK).min(CHUNK);
        let bytes = &mut buffer[..span];
        file.read_exact_at(bytes, offset).map_err(|error| {
            SnapshotError::io(artifact, "rehash checkpoint staging file", &error)
        })?;
        digest.update(bytes);
        offset += u64::try_from(span).expect("the chunk length fits a byte offset");
    }
    if digest.finish() != expected {
        return Err(SnapshotError::StagingDigestMismatch(artifact));
    }
    Ok(())
}
