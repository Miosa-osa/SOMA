//! Resolving a local OCI image layout to the exact manifest one platform selects.
//!
//! This is the honest half of image resolution. Asking a registry which bytes a mutable tag
//! such as `debian:12-slim` names today needs a registry client the workspace does not have,
//! but every flow that builds a Generation already exports the image to a local OCI layout
//! first, and that layout names exactly one manifest per platform. Reading the digest back out
//! of it costs one index walk and no network, and it returns the same manifest the import
//! about to run will select, because both go through the same traversal and verification.

use std::path::Path;

use soma::{OciDigest, OciPlatform};

use crate::{
    ImportError, ImportErrorKind, ImportLimits, ImportPhase, OciSelection, layout::Layout,
    traversal::expected_identity, verify::select_image,
};

/// The exact manifest a local OCI layout holds for one platform.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayoutImage {
    manifest_digest: OciDigest,
    manifest_size: u64,
    platform: OciPlatform,
}

impl LayoutImage {
    /// Returns the digest of the selected OCI manifest.
    #[must_use]
    pub const fn manifest_digest(&self) -> &OciDigest {
        &self.manifest_digest
    }

    /// Returns the byte length of the selected OCI manifest.
    #[must_use]
    pub const fn manifest_size(&self) -> u64 {
        self.manifest_size
    }

    /// Returns the effective platform, refined by the image configuration.
    #[must_use]
    pub const fn platform(&self) -> &OciPlatform {
        &self.platform
    }
}

/// Resolves the unique image a local OCI layout holds for one selection.
///
/// The layout is read but never written, and no blob is copied: only the indexes, the selected
/// manifest, and its configuration are parsed, each within `limits`.
///
/// # Errors
///
/// Returns a redacted [`ImportError`] with [`ImportErrorKind::NotFound`] when the layout holds
/// no image for the selection, [`ImportErrorKind::Ambiguous`] when it holds more than one, and
/// the usual parsing, integrity, and limit failures for a malformed layout.
pub fn resolve_layout_image(
    layout: &Path,
    selection: OciSelection<'_>,
    limits: ImportLimits,
) -> Result<LayoutImage, ImportError> {
    if limits.max_descriptors == 0
        || limits.max_blob_bytes == 0
        || expected_identity(selection).is_some_and(|identity| identity.generation_id().is_some())
    {
        return Err(ImportError::new(
            ImportPhase::OpenLayout,
            ImportErrorKind::InvalidInput,
        ));
    }
    let layout = Layout::open(layout, limits.max_blob_bytes)?;
    let traversal = layout.traverse(selection, limits)?;
    let selected = select_image(&layout, &traversal, selection, limits)?;
    Ok(LayoutImage {
        manifest_digest: selected.candidate.manifest.digest.clone(),
        manifest_size: selected.candidate.manifest.size,
        platform: selected.platform,
    })
}
