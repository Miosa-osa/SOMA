use std::os::unix::fs::FileExt as _;

use super::files::CHUNK;
use super::*;

#[test]
fn mounted_volume_capture_flushes_and_seals_its_own_bytes() {
    use crate::virtio::{BlockRole, FileBackend, TransferShape};

    let device_file = tempfile::tempfile().unwrap();
    let mut logical = vec![0; 2 * CHUNK];
    logical[CHUNK + 9..CHUNK + 15].copy_from_slice(b"volume");
    device_file.write_all_at(&logical, 0).unwrap();

    let mut volume = BlockDevice::new(
        BlockRole::DurableVolume,
        Box::new(FileBackend::new(device_file, false).unwrap()),
        512,
        [0; crate::virtio::BLOCK_SERIAL_LEN],
        TransferShape::Declared,
    )
    .unwrap()
    .into_serving();
    let target = tempfile::tempfile().unwrap();

    let digest = capture_block(Artifact::Volume, Some(&mut volume), Some(&target)).unwrap();

    assert_eq!(digest, Digest::of(&logical));
    let mut recovered = vec![0; logical.len()];
    target.read_exact_at(&mut recovered, 0).unwrap();
    assert_eq!(recovered, logical);
    assert!(target.metadata().unwrap().blocks() * 512 < logical.len() as u64);
    seal(Artifact::Volume, &target, digest).unwrap();
}

#[test]
fn volume_capture_refuses_a_target_shared_with_another_artifact() {
    let state = tempfile::tempfile().unwrap();
    let memory = tempfile::tempfile().unwrap();
    let alias = memory.try_clone().unwrap();
    let error = validate_targets(&CheckpointTargets {
        state: &state,
        memory: &memory,
        overlay: None,
        volume: Some(&alias),
    })
    .unwrap_err();
    assert!(matches!(error, SnapshotError::NotQuiescent(_)));
}

#[test]
fn checkpoint_copy_preserves_zero_spans_and_the_exact_trailing_length() {
    let mut logical = vec![0; 3 * CHUNK + 7];
    logical[CHUNK + 9] = 0x41;
    let file = tempfile::tempfile().unwrap();
    let length = u64::try_from(logical.len()).unwrap();
    let digest = copy_image(Artifact::Overlay, &file, length, |offset, span| {
        let offset = usize::try_from(offset).unwrap();
        span.copy_from_slice(&logical[offset..offset + span.len()]);
        Ok(())
    })
    .unwrap();

    let metadata = file.metadata().unwrap();
    assert_eq!(metadata.len(), length);
    assert!(metadata.blocks() * 512 < length / 2);
    assert_eq!(digest, Digest::of(&logical));
    let mut recovered = vec![0; logical.len()];
    file.read_exact_at(&mut recovered, 0).unwrap();
    assert_eq!(recovered, logical);
    seal(Artifact::Overlay, &file, digest).unwrap();
}
