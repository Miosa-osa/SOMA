use std::{fs::File, path::Path};

use soma::GenerationId;

use super::{
    artifacts::{ArtifactDescriptor, ArtifactRole, Sha256Digest},
    candidate::{CandidateId, Certification, PublishedCandidate, PublishedGeneration},
    error::{CompileError, CompileErrorKind, CompilePhase},
    identity::derive_generation_id,
    manifest::{
        GenerationManifest, MAX_MANIFEST_BYTES, SnapshotBinding, encode_candidate, encode_manifest,
    },
};
use crate::{ImportPhase, store::Store};

/// Publishes the Candidate manifest as the last object of a Generation build.
///
/// Every artifact the manifest references must already be published.
/// The Candidate object is linked into the store with create-exclusive semantics; an existing
/// object with the same digest is re-verified byte for byte rather than overwritten, so two
/// concurrent identical builds converge and a differing byte sequence fails closed.
/// Nothing published here is resolvable as a Generation: the bytes carry the Candidate magic
/// and the identity is a `CandidateId`.
pub(crate) fn publish_candidate(
    store: &Store,
    manifest: &GenerationManifest,
) -> Result<PublishedCandidate, CompileError> {
    if manifest.snapshot != SnapshotBinding::Absent {
        return Err(CompileError::new(
            CompilePhase::Publish,
            CompileErrorKind::InvalidInput,
        ));
    }
    let bytes = encode_candidate(manifest)?;
    let descriptor = link_last(store, manifest, &bytes, ArtifactRole::GenerationCandidate)?;
    Ok(PublishedCandidate {
        id: CandidateId::of(&bytes),
        descriptor,
        manifest: manifest.clone(),
    })
}

/// Publishes the ready Generation manifest as the last object of a certified build.
///
/// Only [`Certification`] reaches this function, so no failure before certification can leave a
/// ready Generation identity in the store.
pub(crate) fn publish_certified(
    store: &Store,
    manifest: &GenerationManifest,
    certification: &Certification,
) -> Result<PublishedGeneration, CompileError> {
    let mut ready = manifest.clone();
    ready.snapshot = certification.snapshot();
    if ready.snapshot == SnapshotBinding::Absent {
        return Err(CompileError::new(
            CompilePhase::Publish,
            CompileErrorKind::InvalidInput,
        ));
    }
    let bytes = encode_manifest(&ready)?;
    let descriptor = link_last(store, &ready, &bytes, ArtifactRole::GenerationManifest)?;
    Ok(PublishedGeneration {
        id: derive_generation_id(&bytes),
        descriptor,
        manifest: ready,
    })
}

/// Requires every referenced artifact to exist, then links the manifest object last.
fn link_last(
    store: &Store,
    manifest: &GenerationManifest,
    bytes: &[u8],
    role: ArtifactRole,
) -> Result<ArtifactDescriptor, CompileError> {
    for descriptor in manifest.descriptors() {
        store
            .open_blob(&descriptor.to_store_descriptor(), ImportPhase::Publish)
            .map_err(|error| CompileError::from_import(CompilePhase::Publish, error))?;
    }
    let stored = store
        .put_bytes(bytes, role.media_type(), ImportPhase::Publish)
        .map_err(|error| CompileError::from_import(CompilePhase::Publish, error))?;
    Ok(ArtifactDescriptor {
        role,
        digest: Sha256Digest::from_oci(&stored.digest),
        size: stored.size,
    })
}

/// Opens and digest-verifies one published Generation artifact by its manifest descriptor.
///
/// The returned handle is the same handle whose bytes passed digest and size verification.
/// This matters at the launch boundary: verifying one path and reopening it later would allow
/// a mutable host directory to substitute different bytes between those two operations.
/// Callers can hand this handle directly to a machine without composing a host path or trusting
/// a second lookup.
///
/// # Errors
///
/// Returns a redacted [`CompileError`] when the store or the object cannot be opened or the
/// object length disagrees with the descriptor.
pub fn open_artifact(store: &Path, descriptor: &ArtifactDescriptor) -> Result<File, CompileError> {
    let store = Store::open(store)
        .map_err(|error| CompileError::from_import(CompilePhase::VerifyGeneration, error))?;
    let file = store
        .open_verified_blob(
            &descriptor.to_store_descriptor(),
            descriptor.size,
            ImportPhase::Publish,
        )
        .map_err(|error| CompileError::from_import(CompilePhase::VerifyGeneration, error))?;
    Ok(file.into_std())
}

/// Reads and digest-checks the ready manifest bytes named by a `GenerationId`.
pub(crate) fn read_manifest_bytes(
    store: &Store,
    id: &GenerationId,
) -> Result<Vec<u8>, CompileError> {
    read_object(store, super::identity::generation_id_digest(id))
}

/// Reads and digest-checks the Candidate manifest bytes named by a `CandidateId`.
pub(crate) fn read_candidate_bytes(
    store: &Store,
    id: &CandidateId,
) -> Result<Vec<u8>, CompileError> {
    read_object(store, id.digest())
}

fn read_object(store: &Store, digest: Sha256Digest) -> Result<Vec<u8>, CompileError> {
    let bytes = store
        .read_bounded(&digest.to_oci(), MAX_MANIFEST_BYTES, ImportPhase::Publish)
        .map_err(|error| CompileError::from_import(CompilePhase::VerifyGeneration, error))?;
    if Sha256Digest::of(&bytes) != digest {
        return Err(CompileError::new(
            CompilePhase::VerifyGeneration,
            CompileErrorKind::Integrity,
        ));
    }
    Ok(bytes)
}
