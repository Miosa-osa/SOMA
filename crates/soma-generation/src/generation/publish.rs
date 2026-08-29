use std::{fs::File, path::Path};

use soma::GenerationId;

use super::{
    artifacts::{ArtifactDescriptor, ArtifactRole, Sha256Digest},
    error::{CompileError, CompilePhase},
    identity::derive_generation_id,
    manifest::{GenerationManifest, MAX_MANIFEST_BYTES, SnapshotBinding, encode_manifest},
};
use crate::{ImportPhase, store::Store};

/// One Generation manifest that has been published under its identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedGeneration {
    /// The identity derived from the exact manifest bytes.
    pub id: GenerationId,
    /// The manifest artifact descriptor.
    pub descriptor: ArtifactDescriptor,
    /// The decoded manifest that was published.
    pub manifest: GenerationManifest,
}

impl PublishedGeneration {
    /// Returns whether a certified snapshot is bound; without one the Generation cannot Launch.
    #[must_use]
    pub const fn launchable(&self) -> bool {
        matches!(self.manifest.snapshot, SnapshotBinding::Captured { .. })
    }
}

/// Publishes the canonical manifest as the last object of a Generation build.
///
/// Every artifact the manifest references must already be published.
/// The manifest object is linked into the store with create-exclusive semantics; an existing
/// object with the same digest is re-verified byte for byte rather than overwritten, so two
/// concurrent identical builds converge and a differing byte sequence fails closed.
pub(crate) fn publish_manifest(
    store: &Store,
    manifest: &GenerationManifest,
) -> Result<PublishedGeneration, CompileError> {
    let bytes = encode_manifest(manifest)?;
    for descriptor in manifest.descriptors() {
        store
            .open_blob(&descriptor.to_store_descriptor(), ImportPhase::Publish)
            .map_err(|error| CompileError::from_import(CompilePhase::Publish, error))?;
    }
    let stored = store
        .put_bytes(
            &bytes,
            ArtifactRole::GenerationManifest.media_type(),
            ImportPhase::Publish,
        )
        .map_err(|error| CompileError::from_import(CompilePhase::Publish, error))?;
    let descriptor = ArtifactDescriptor {
        role: ArtifactRole::GenerationManifest,
        digest: Sha256Digest::from_oci(&stored.digest),
        size: stored.size,
    };
    Ok(PublishedGeneration {
        id: derive_generation_id(&bytes),
        descriptor,
        manifest: manifest.clone(),
    })
}

/// Opens one published Generation artifact read-only by its manifest descriptor.
///
/// The store rejects a symlink, and the open file's length must equal the descriptor size.
/// Callers that need digest proof read the file through their own hasher; this accessor
/// exists so a launcher can hand the immutable root, overlay template, kernel, and initramfs
/// to a machine without ever composing a host path from artifact identity.
///
/// # Errors
///
/// Returns a redacted [`CompileError`] when the store or the object cannot be opened or the
/// object length disagrees with the descriptor.
pub fn open_artifact(store: &Path, descriptor: &ArtifactDescriptor) -> Result<File, CompileError> {
    let store = Store::open(store)
        .map_err(|error| CompileError::from_import(CompilePhase::VerifyGeneration, error))?;
    let file = store
        .open_blob(&descriptor.to_store_descriptor(), ImportPhase::Publish)
        .map_err(|error| CompileError::from_import(CompilePhase::VerifyGeneration, error))?;
    let length = file
        .metadata()
        .map_err(|_| {
            CompileError::new(
                CompilePhase::VerifyGeneration,
                super::error::CompileErrorKind::Io,
            )
        })?
        .len();
    if length != descriptor.size {
        return Err(CompileError::new(
            CompilePhase::VerifyGeneration,
            super::error::CompileErrorKind::Integrity,
        ));
    }
    Ok(file.into_std())
}

/// Reads and digest-checks the manifest bytes named by a `GenerationId`.
pub(crate) fn read_manifest_bytes(
    store: &Store,
    id: &GenerationId,
) -> Result<Vec<u8>, CompileError> {
    let digest = super::identity::generation_id_digest(id);
    let bytes = store
        .read_bounded(&digest.to_oci(), MAX_MANIFEST_BYTES, ImportPhase::Publish)
        .map_err(|error| CompileError::from_import(CompilePhase::VerifyGeneration, error))?;
    if Sha256Digest::of(&bytes) != digest {
        return Err(CompileError::new(
            CompilePhase::VerifyGeneration,
            super::error::CompileErrorKind::Integrity,
        ));
    }
    Ok(bytes)
}
