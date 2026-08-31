use crate::{
    ImportError, ImportErrorKind, ImportOciLayout, ImportPhase, layout::Layout, publish,
    traversal::expected_identity, verify::select_image,
};

/// Imports one verified OCI image from a local OCI image-layout into an immutable CAS.
///
/// # Errors
///
/// Returns a redacted [`ImportError`] when selection, verification, limits, or publication fails.
pub fn import_oci_layout(request: ImportOciLayout<'_>) -> Result<crate::ImportedOci, ImportError> {
    validate_request(request)?;
    let layout = Layout::open(request.layout, request.limits.max_blob_bytes)?;
    let traversal = layout.traverse(request.selection, request.limits)?;
    let selected = select_image(&layout, &traversal, request.selection, request.limits)?;
    publish::publish(&layout, traversal, selected, request)
}

fn validate_request(request: ImportOciLayout<'_>) -> Result<(), ImportError> {
    let limits = request.limits;
    if limits.max_descriptors == 0
        || limits.max_blob_bytes == 0
        || limits.max_total_blob_bytes == 0
        || limits.max_expanded_bytes == 0
        || expected_identity(request.selection)
            .is_some_and(|identity| identity.generation_id().is_some())
    {
        return Err(ImportError::new(
            ImportPhase::OpenLayout,
            ImportErrorKind::InvalidInput,
        ));
    }
    Ok(())
}
