//! An OCI resolver backed by a local OCI image layout.

use std::path::{Path, PathBuf};

use soma::{OciImage, OciPlatform};
use soma_template::{OciResolver, ResolveError, ResolvedImage};

use crate::{ImportErrorKind, ImportLimits, OciSelection, layout_image::resolve_layout_image};

/// Resolves one image reference from the local OCI layout it was exported to.
///
/// A registry client would answer for any reference; this resolver answers for exactly the one
/// reference the layout was exported for, and refuses every other. That refusal is the point:
/// a layout directory carries no trustworthy record of which reference produced it, because
/// `skopeo copy docker://<reference> oci:<dir>:<tag>` annotates the index with the local tag
/// rather than the source reference. Binding the reference at construction keeps the resolver
/// from answering a question it cannot actually check.
pub struct LayoutResolver {
    layout: PathBuf,
    reference: OciImage,
    limits: ImportLimits,
}

impl LayoutResolver {
    /// Resolves `reference` from the OCI layout at `layout`, within `limits`.
    #[must_use]
    pub fn new(layout: &Path, reference: &OciImage, limits: ImportLimits) -> Self {
        Self {
            layout: layout.to_path_buf(),
            reference: reference.clone(),
            limits,
        }
    }

    /// Returns the reference this resolver answers for.
    #[must_use]
    pub const fn reference(&self) -> &OciImage {
        &self.reference
    }
}

impl OciResolver for LayoutResolver {
    fn resolve(
        &self,
        reference: &OciImage,
        platform: &OciPlatform,
    ) -> Result<ResolvedImage, ResolveError> {
        if reference.as_str() != self.reference.as_str() {
            return Err(ResolveError::Unresolvable);
        }
        let image =
            resolve_layout_image(&self.layout, OciSelection::Platform(platform), self.limits)
                .map_err(|error| match error.kind() {
                    // A layout that holds no image for the platform, or more than one, is a resolution
                    // answer rather than an infrastructure failure: the reference names nothing exact.
                    ImportErrorKind::NotFound | ImportErrorKind::Ambiguous => {
                        ResolveError::Unresolvable
                    }
                    _ => ResolveError::Unavailable(format!("reading the OCI layout: {error}")),
                })?;
        Ok(ResolvedImage::new(
            image.manifest_digest().clone(),
            image.platform().clone(),
            image.manifest_size(),
        ))
    }
}
