//! Retained-handle verification of a published snapshot before admission.

use std::{fs::File, io::Read as _, path::Path};

use super::{
    artifacts::{SnapshotPaths, hash},
    error::{Artifact, SnapshotError},
};
use crate::snapshot::inspection::{
    ArtifactEvidence, CaptureExpectation, VerifiedCapture, inspect_capture,
};

/// Enough for the bounded schema-v2 header and 32 maximum-sized sections.
const MAX_STATE_BYTES: u64 = 34 * 1024 * 1024;

/// Reopens and verifies the complete published capture through retained read-only handles.
///
/// This installation operation hashes all three artifacts and is never part of warm Launch.
///
/// # Errors
///
/// Returns the first file, schema, Candidate, size, or digest mismatch.
pub fn inspect(
    paths: &SnapshotPaths,
    expected: CaptureExpectation,
) -> Result<VerifiedCapture, SnapshotError> {
    let mut memory = open(Artifact::Memory, &paths.memory())?;
    let mut overlay = open(Artifact::Overlay, &paths.overlay())?;
    let mut state = open(Artifact::State, &paths.state())?;
    let memory_evidence = evidence(Artifact::Memory, &mut memory)?;
    let overlay_evidence = evidence(Artifact::Overlay, &mut overlay)?;
    let state_size = state
        .metadata()
        .map_err(|error| SnapshotError::io(Artifact::State, "stat", &error))?
        .len();
    if state_size > MAX_STATE_BYTES {
        return Err(SnapshotError::ArtifactTooLarge {
            artifact: Artifact::State,
            size: state_size,
            maximum: MAX_STATE_BYTES,
        });
    }
    let mut state_bytes = Vec::with_capacity(usize::try_from(state_size).unwrap_or(0));
    state
        .read_to_end(&mut state_bytes)
        .map_err(|error| SnapshotError::io(Artifact::State, "read", &error))?;
    Ok(inspect_capture(
        &state_bytes,
        memory_evidence,
        overlay_evidence,
        expected,
    )?)
}

fn open(artifact: Artifact, path: &Path) -> Result<File, SnapshotError> {
    File::open(path).map_err(|error| SnapshotError::io(artifact, "open for inspection", &error))
}

fn evidence(artifact: Artifact, file: &mut File) -> Result<ArtifactEvidence, SnapshotError> {
    let size = file
        .metadata()
        .map_err(|error| SnapshotError::io(artifact, "stat", &error))?
        .len();
    let digest = hash(artifact, file)?;
    Ok(ArtifactEvidence { digest, size })
}
