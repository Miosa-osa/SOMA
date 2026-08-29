use std::io::Read as _;

use crate::{
    ImportPhase, NormalizeError, NormalizeErrorKind, NormalizeOciRootfs, NormalizePhase, digest,
    manifest::{self, DecodedImport, IMPORT_MEDIA_TYPE},
    oci::Descriptor,
    store::Store,
};

pub(super) fn reopen(
    request: NormalizeOciRootfs<'_>,
) -> Result<(Store, DecodedImport), NormalizeError> {
    validate_limits(request)?;
    let store = Store::open(request.store)
        .map_err(|error| NormalizeError::from_import(NormalizePhase::OpenImport, error))?;
    let descriptor = Descriptor {
        media_type: IMPORT_MEDIA_TYPE.to_owned(),
        digest: request.imported.import_manifest_digest.clone(),
        size: request.imported.import_manifest_size,
        platform: None,
    };
    let bytes = read_manifest(&store, &descriptor, request.limits.max_manifest_bytes)?;
    let imported = manifest::decode(&bytes)?;
    if imported.workload != request.imported.workload {
        return Err(error(NormalizeErrorKind::Integrity));
    }
    validate_records(&imported, request)?;
    Ok((store, imported))
}

fn read_manifest(
    store: &Store,
    descriptor: &Descriptor,
    maximum: u64,
) -> Result<Vec<u8>, NormalizeError> {
    let mut file = store
        .open_verified_blob(descriptor, maximum, ImportPhase::Publish)
        .map_err(|error| NormalizeError::from_import(NormalizePhase::OpenImport, error))?;
    let capacity =
        usize::try_from(descriptor.size).map_err(|_| error(NormalizeErrorKind::LimitExceeded))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.read_to_end(&mut bytes)
        .map_err(|_| error(NormalizeErrorKind::Io))?;
    if bytes.len() != capacity || digest::bytes(&bytes) != descriptor.digest {
        return Err(error(NormalizeErrorKind::Integrity));
    }
    Ok(bytes)
}

fn validate_limits(request: NormalizeOciRootfs<'_>) -> Result<(), NormalizeError> {
    let limits = request.limits;
    if limits.max_blob_bytes == 0
        || limits.max_expanded_bytes == 0
        || limits.max_entries == 0
        || limits.max_path_bytes == 0
        || limits.max_metadata_bytes == 0
        || limits.max_file_bytes == 0
        || limits.max_content_bytes == 0
        || limits.max_manifest_bytes == 0
        || request.imported.workload.generation_id().is_some()
    {
        return Err(error(NormalizeErrorKind::InvalidInput));
    }
    if request.imported.import_manifest_size > limits.max_manifest_bytes {
        return Err(error(NormalizeErrorKind::LimitExceeded));
    }
    Ok(())
}

fn validate_records(
    imported: &DecodedImport,
    request: NormalizeOciRootfs<'_>,
) -> Result<(), NormalizeError> {
    let mut expanded = 0_u64;
    let mut entries = 0_u32;
    for layer in &imported.layers {
        if layer.descriptor.size > request.limits.max_blob_bytes {
            return Err(error(NormalizeErrorKind::LimitExceeded));
        }
        expanded = expanded
            .checked_add(layer.expanded_size)
            .ok_or_else(|| error(NormalizeErrorKind::LimitExceeded))?;
        entries = entries
            .checked_add(layer.entry_count)
            .ok_or_else(|| error(NormalizeErrorKind::LimitExceeded))?;
    }
    if expanded > request.limits.max_expanded_bytes || entries > request.limits.max_entries {
        return Err(error(NormalizeErrorKind::LimitExceeded));
    }
    Ok(())
}

const fn error(kind: NormalizeErrorKind) -> NormalizeError {
    NormalizeError::new(NormalizePhase::OpenImport, kind)
}
