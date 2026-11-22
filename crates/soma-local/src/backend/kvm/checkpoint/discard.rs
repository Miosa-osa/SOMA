//! Durable, identity-bound removal of a checkpoint whose consumer has finished.

use std::{fs, io::ErrorKind, path::Path};

use soma::{GenerationId, InstanceId, OperationId};

use crate::{CheckpointFailure, CheckpointReceipt};

use super::{
    MEMORY, OVERLAY, RECEIPT, STATE, configured_root, read_bounded, read_published, rename_new,
    sync_directory,
};

pub(crate) fn discard_checkpoint(
    instance: &InstanceId,
    checkpoint: &OperationId,
    state_digest: &GenerationId,
) -> Result<(), CheckpointFailure> {
    discard_from(&configured_root()?, instance, checkpoint, state_digest)
}

fn discard_from(
    root: &Path,
    instance: &InstanceId,
    checkpoint: &OperationId,
    state_digest: &GenerationId,
) -> Result<(), CheckpointFailure> {
    let key = format!("{}.{}", instance.as_str(), checkpoint.as_str());
    let published = root.join(&key);
    let retired = root.join(format!(".discarded-{key}"));

    if directory_exists(&published)? {
        let receipt: CheckpointReceipt =
            serde_json::from_slice(&read_bounded(&published.join(RECEIPT))?)
                .map_err(|_| CheckpointFailure::Storage)?;
        if receipt.state_digest != state_digest.as_str() {
            return Err(CheckpointFailure::CaptureRefused);
        }
        let receipt = read_published(&published, instance, &receipt.generation_id, checkpoint)?
            .ok_or(CheckpointFailure::Unconfirmed)?;
        if receipt.state_digest != state_digest.as_str() {
            return Err(CheckpointFailure::CaptureRefused);
        }
        rename_new(&published, &retired)?;
        sync_directory(root)?;
    }

    if directory_exists(&retired)? {
        // The receipt stays until everything else is gone, allowing a retry
        // after a partial removal to check that it still has the exact target.
        let receipt = retired.join(RECEIPT);
        if receipt
            .try_exists()
            .map_err(|_| CheckpointFailure::Storage)?
        {
            let bound: CheckpointReceipt = serde_json::from_slice(&read_bounded(&receipt)?)
                .map_err(|_| CheckpointFailure::Storage)?;
            if &bound.instance_id != instance
                || &bound.checkpoint_id != checkpoint
                || bound.state_digest != state_digest.as_str()
            {
                return Err(CheckpointFailure::CaptureRefused);
            }
        } else if fs::read_dir(&retired)
            .map_err(|_| CheckpointFailure::Storage)?
            .next()
            .is_some()
        {
            return Err(CheckpointFailure::Storage);
        }

        for name in [STATE, MEMORY, OVERLAY, RECEIPT] {
            match fs::remove_file(retired.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(_) => return Err(CheckpointFailure::Storage),
            }
        }
        fs::remove_dir(&retired).map_err(|_| CheckpointFailure::Storage)?;
    }

    sync_directory(root)
}

fn directory_exists(path: &Path) -> Result<bool, CheckpointFailure> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        _ => Err(CheckpointFailure::Storage),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt as _};

    use super::*;

    fn identity() -> (InstanceId, OperationId, GenerationId) {
        (
            InstanceId::new("11".repeat(16)).unwrap(),
            OperationId::new("22".repeat(16)).unwrap(),
            GenerationId::new(format!("sha256:{}", "33".repeat(32))).unwrap(),
        )
    }

    #[test]
    fn a_lost_acknowledgement_can_repeat_cleanup_of_an_absent_checkpoint() {
        let directory = tempfile::tempdir().unwrap();
        let (instance, checkpoint, digest) = identity();
        assert_eq!(
            discard_from(directory.path(), &instance, &checkpoint, &digest),
            Ok(())
        );
    }

    #[test]
    fn a_partial_removal_replays_only_with_its_original_digest() {
        let directory = tempfile::tempdir().unwrap();
        let (instance, checkpoint, digest) = identity();
        let retired = directory.path().join(format!(
            ".discarded-{}.{}",
            instance.as_str(),
            checkpoint.as_str()
        ));
        fs::create_dir(&retired).unwrap();
        fs::write(retired.join(MEMORY), b"partial").unwrap();
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(retired.join(RECEIPT))
            .unwrap();
        file.write_all(
            &serde_json::to_vec(&CheckpointReceipt {
                instance_id: instance.clone(),
                checkpoint_id: checkpoint.clone(),
                generation_id: digest.clone(),
                state_digest: digest.as_str().to_owned(),
                memory_digest: digest.as_str().to_owned(),
                memory_bytes: 7,
                overlay_bytes: 0,
                overlay_digest: None,
            })
            .unwrap(),
        )
        .unwrap();
        let wrong = GenerationId::new(format!("sha256:{}", "44".repeat(32))).unwrap();
        assert_eq!(
            discard_from(directory.path(), &instance, &checkpoint, &wrong),
            Err(CheckpointFailure::CaptureRefused),
        );
        assert!(retired.join(MEMORY).is_file());
        assert_eq!(
            discard_from(directory.path(), &instance, &checkpoint, &digest),
            Ok(())
        );
        assert!(!retired.exists());
    }

    #[test]
    fn a_tombstone_without_its_receipt_can_only_be_empty() {
        let directory = tempfile::tempdir().unwrap();
        let (instance, checkpoint, digest) = identity();
        let retired = directory.path().join(format!(
            ".discarded-{}.{}",
            instance.as_str(),
            checkpoint.as_str()
        ));
        fs::create_dir(&retired).unwrap();
        fs::write(retired.join(MEMORY), b"orphan").unwrap();
        assert_eq!(
            discard_from(directory.path(), &instance, &checkpoint, &digest),
            Err(CheckpointFailure::Storage),
        );
        assert!(retired.join(MEMORY).is_file());
    }
}
