//! Proving an immutable restore input still holds the bytes its record names.

use std::fs::{File, Metadata};

use super::SnapshotError;
use crate::{
    snapshot::{ArtifactAttestation, Digest, FileStamp, memory::MemoryDescriptor},
    x86_64::snapshot::{Artifact, artifacts::hash},
};

/// The digest of `file`, read in full unless the broker's attestation vouches for this exact
/// descriptor, size, and `expected` digest.
///
/// An attestation that names another stamp, size, or digest is ignored rather than refused: the
/// file is then hashed exactly as without one, so an attestation can only skip a read, never
/// admit bytes the record does not name. The caller still compares the result with its record.
///
/// # Errors
///
/// Returns the read failure.
pub(super) fn digest(
    artifact: Artifact,
    file: &mut File,
    metadata: &Metadata,
    attestation: Option<ArtifactAttestation>,
    expected: Digest,
) -> Result<Digest, SnapshotError> {
    let vouched = attestation.and_then(|attestation| {
        attestation.vouches(FileStamp::of(metadata), metadata.len(), expected)
    });
    match vouched {
        Some(digest) => Ok(digest),
        None => hash(artifact, file),
    }
}

/// Checks the memory file against its manifest record.
///
/// # Errors
///
/// Returns the stat or read failure, a non-regular file, or the size or digest mismatch.
pub(super) fn verify_memory(
    file: &mut File,
    record: &MemoryDescriptor,
    attestation: Option<ArtifactAttestation>,
) -> Result<(), SnapshotError> {
    let metadata = file
        .metadata()
        .map_err(|error| SnapshotError::io(Artifact::Memory, "stat", &error))?;
    if !metadata.is_file() {
        return Err(SnapshotError::NotQuiescent(
            "checkpoint restore requires regular files",
        ));
    }
    let digest = digest(
        Artifact::Memory,
        file,
        &metadata,
        attestation,
        record.digest(),
    )?;
    record.verify_generation(digest, metadata.len())?;
    Ok(())
}

#[cfg(test)]
mod tests;
