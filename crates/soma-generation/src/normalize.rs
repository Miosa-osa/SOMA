mod entry;
mod error;
mod layer;
mod node_plan;
mod pax;
mod source;
mod stream;
mod tree;
mod tree_manifest;
mod tree_model;
mod types;

pub use error::{NormalizeError, NormalizeErrorKind, NormalizePhase};
pub use types::{NormalizeOciRootfs, NormalizedRootfs, RootfsLimits};

pub(crate) use node_plan::CONTENT_MEDIA_TYPE;
pub(crate) use tree_manifest::MEDIA_TYPE as TREE_MEDIA_TYPE;

/// Converts verified OCI layers into one deterministic logical rootfs tree artifact.
///
/// # Errors
///
/// Returns a redacted [`NormalizeError`] when stored input, layer semantics, configured limits,
/// or immutable publication fails.
pub fn normalize_oci_rootfs(
    request: NormalizeOciRootfs<'_>,
) -> Result<NormalizedRootfs, NormalizeError> {
    let (store, imported) = source::reopen(request)?;
    let mut tree = tree::Tree::new(request.limits)?;
    let mut budget = layer::Budget::new();
    let mut preflight_budget = crate::tar_preflight::PreflightBudget::new(
        request.limits.max_entries,
        request.limits.max_metadata_bytes,
    );
    for record in &imported.layers {
        let plan = layer::parse(
            &store,
            record,
            request.limits,
            &mut budget,
            &mut preflight_budget,
        )?;
        tree.apply(plan)?;
    }
    let stats = tree.stats()?;
    let manifest = tree_manifest::encode(&tree, request.limits.max_manifest_bytes)?;
    let descriptor = store
        .put_bytes(
            &manifest,
            tree_manifest::MEDIA_TYPE,
            crate::ImportPhase::Publish,
        )
        .map_err(|error| NormalizeError::from_import(NormalizePhase::Publish, error))?;
    Ok(NormalizedRootfs {
        workload: request.imported.workload.clone(),
        source_import_manifest_digest: request.imported.import_manifest_digest.clone(),
        tree_manifest_digest: descriptor.digest,
        tree_manifest_size: descriptor.size,
        entry_count: stats.entry_count,
        logical_file_bytes: stats.logical_file_bytes,
        content_blob_count: stats.content_blob_count,
        content_blob_bytes: stats.content_blob_bytes,
    })
}
