//! Holds derived from the same private namespace that publishes checkpoint receipts.

use std::{fs, path::Path};

use soma::{GenerationId, InstanceId, OperationId};

use crate::CheckpointFailure;

use super::{RECEIPT, configured_root, read_bounded, read_published};

pub(in crate::backend) fn retains_stopped_checkpoint(
    instance: &InstanceId,
    generation: &GenerationId,
) -> Result<bool, CheckpointFailure> {
    let mut retained = false;
    visit(Some(instance), |path, instance, operation| {
        retained = read_published(path, instance, generation, operation)?.is_some();
        Ok(!retained)
    })?;
    Ok(retained)
}

pub(in crate::backend::kvm) fn generation_holds()
-> Result<Vec<(InstanceId, GenerationId)>, CheckpointFailure> {
    let mut holds = Vec::new();
    visit(None, |path, instance, operation| {
        let receipt: crate::CheckpointReceipt =
            serde_json::from_slice(&read_bounded(&path.join(RECEIPT))?)
                .map_err(|_| CheckpointFailure::Storage)?;
        let generation = receipt.generation_id;
        if read_published(path, instance, &generation, operation)?.is_some() {
            holds.push((instance.clone(), generation));
        }
        Ok(true)
    })?;
    holds.sort_unstable_by(|a, b| (a.0.as_str(), a.1.as_str()).cmp(&(b.0.as_str(), b.1.as_str())));
    holds.dedup();
    Ok(holds)
}

/// The checkpoints `instance` has published, with the state digest each was published under.
///
/// A destroyed Instance can never be restored or forked again, so its checkpoints are discarded
/// with it rather than left on the encrypted root for nothing to ever reclaim.
pub(crate) fn published_checkpoints(
    instance: &InstanceId,
) -> Result<Vec<(OperationId, GenerationId)>, CheckpointFailure> {
    match configured_root() {
        Ok(root) => published_in(&root, instance),
        Err(CheckpointFailure::Unsupported) => Ok(Vec::new()),
        Err(failure) => Err(failure),
    }
}

fn published_in(
    root: &Path,
    instance: &InstanceId,
) -> Result<Vec<(OperationId, GenerationId)>, CheckpointFailure> {
    let mut published = Vec::new();
    visit_in(root, Some(instance), |path, _instance, operation| {
        let receipt: crate::CheckpointReceipt =
            serde_json::from_slice(&read_bounded(&path.join(RECEIPT))?)
                .map_err(|_| CheckpointFailure::Storage)?;
        let digest =
            GenerationId::new(receipt.state_digest).map_err(|_| CheckpointFailure::Storage)?;
        published.push((operation.clone(), digest));
        Ok(true)
    })?;
    Ok(published)
}

fn visit(
    filter: Option<&InstanceId>,
    publication: impl FnMut(&Path, &InstanceId, &OperationId) -> Result<bool, CheckpointFailure>,
) -> Result<(), CheckpointFailure> {
    match configured_root() {
        Ok(root) => visit_in(&root, filter, publication),
        Err(CheckpointFailure::Unsupported) => Ok(()),
        Err(failure) => Err(failure),
    }
}

fn visit_in(
    root: &Path,
    filter: Option<&InstanceId>,
    mut publication: impl FnMut(&Path, &InstanceId, &OperationId) -> Result<bool, CheckpointFailure>,
) -> Result<(), CheckpointFailure> {
    for result in fs::read_dir(root).map_err(|_| CheckpointFailure::Storage)? {
        let entry = result.map_err(|_| CheckpointFailure::Storage)?;
        let file_name = entry.file_name();
        let name = file_name.to_str().ok_or(CheckpointFailure::Storage)?;
        if name.starts_with(".staging-") || name.starts_with(".discarded-") {
            continue;
        }
        let (instance, operation) = name.split_once('.').ok_or(CheckpointFailure::Storage)?;
        if filter.is_some_and(|filter| filter.as_str() != instance) {
            continue;
        }
        let instance = InstanceId::new(instance).map_err(|_| CheckpointFailure::Storage)?;
        let operation = OperationId::new(operation).map_err(|_| CheckpointFailure::Storage)?;
        if !entry
            .file_type()
            .map_err(|_| CheckpointFailure::Storage)?
            .is_dir()
        {
            return Err(CheckpointFailure::Storage);
        }
        if !publication(&entry.path(), &instance, &operation)? {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt as _};

    use soma::{GenerationId, InstanceId, OperationId};

    use super::published_in;

    fn publish(root: &std::path::Path, instance: &str, operation: &str, digest: &str) {
        let directory = root.join(format!("{instance}.{operation}"));
        fs::create_dir(&directory).expect("publication");
        let receipt = serde_json::json!({
            "instance_id": instance,
            "checkpoint_id": operation,
            "generation_id": format!("sha256:{}", "1".repeat(64)),
            "state_digest": digest,
            "memory_digest": format!("sha256:{}", "2".repeat(64)),
            "memory_bytes": 4096,
            "overlay_bytes": 0,
            "overlay_digest": null,
        });
        fs::write(
            directory.join("receipt.json"),
            serde_json::to_vec(&receipt).expect("receipt"),
        )
        .expect("receipt written");
        // Published checkpoints are private, and the reader refuses anything else.
        fs::set_permissions(
            directory.join("receipt.json"),
            fs::Permissions::from_mode(0o600),
        )
        .expect("private receipt");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).expect("private dir");
    }

    #[test]
    fn a_destroyed_instance_finds_every_checkpoint_it_published_and_no_other() {
        let root = tempfile::tempdir().expect("root");
        let mine = "a".repeat(32);
        let other = "b".repeat(32);
        let digest = |n: &str| format!("sha256:{}", n.repeat(64));
        publish(root.path(), &mine, &"c".repeat(32), &digest("3"));
        publish(root.path(), &mine, &"d".repeat(32), &digest("4"));
        publish(root.path(), &other, &"e".repeat(32), &digest("5"));
        fs::create_dir(root.path().join(".staging-0123")).expect("a stage is skipped");

        let mut found =
            published_in(root.path(), &InstanceId::new(&mine).expect("instance")).expect("listed");
        found.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        assert_eq!(
            found,
            vec![
                (
                    OperationId::new("c".repeat(32)).expect("op"),
                    GenerationId::new(digest("3")).expect("digest")
                ),
                (
                    OperationId::new("d".repeat(32)).expect("op"),
                    GenerationId::new(digest("4")).expect("digest")
                ),
            ]
        );
    }
}
