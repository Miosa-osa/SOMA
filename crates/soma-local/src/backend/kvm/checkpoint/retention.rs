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

fn visit(
    filter: Option<&InstanceId>,
    mut publication: impl FnMut(&Path, &InstanceId, &OperationId) -> Result<bool, CheckpointFailure>,
) -> Result<(), CheckpointFailure> {
    let root = match configured_root() {
        Ok(root) => root,
        Err(CheckpointFailure::Unsupported) => return Ok(()),
        Err(failure) => return Err(failure),
    };
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
