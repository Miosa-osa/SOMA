//! The gate between a Candidate and a ready Generation.
//!
//! A Candidate becomes a Generation only after boot and capture, compatibility, security, and
//! certification all succeed.
//! [`promote_candidate`] is the only publication path for a ready manifest and it requires the
//! [`Certification`] token, so a revoked or failed Candidate cannot be promoted without running
//! the gates again.

use std::{io::Read as _, path::Path};

use soma_kvm::snapshot::{
    Digest,
    inspection::{ArtifactEvidence, CaptureExpectation, inspect_capture},
    manifest::{CandidateId as SnapshotCandidateId, Manifest as SnapshotManifest},
};

use super::{
    SnapshotBinding,
    candidate::{Certification, PublishedCandidate, PublishedGeneration},
    error::{CompileError, CompileErrorKind, CompilePhase},
    publish::publish_certified,
    request::CompilerProfile,
    verify::verify_candidate,
};
use crate::store::Store;

const MAX_SNAPSHOT_STATE_BYTES: u64 = 34 * 1024 * 1024;

/// Runs the boot, capture, compatibility, security, and certification gates for one Candidate.
///
/// # Errors
///
/// Every other failure returns the phase that rejected the Candidate.
pub fn certify_candidate(
    store: &Path,
    candidate: &PublishedCandidate,
    profile: &CompilerProfile,
    snapshot: SnapshotBinding,
) -> Result<Certification, CompileError> {
    // Cross-artifact verification runs first so a Candidate that cannot re-verify never reaches
    // snapshot certification.
    let verified = verify_candidate(store, &candidate.id, profile)?;
    if verified.manifest != candidate.manifest {
        return Err(CompileError::new(
            CompilePhase::VerifyGeneration,
            CompileErrorKind::Integrity,
        ));
    }
    let store = Store::open(store)
        .map_err(|error| CompileError::from_import(CompilePhase::Certify, error))?;
    verify_snapshot_binding(&store, snapshot, Some(&candidate.id), CompilePhase::Certify)?;
    Ok(Certification::new(candidate.id.clone(), snapshot))
}

pub(crate) fn verify_snapshot_binding(
    store: &Store,
    snapshot: SnapshotBinding,
    candidate: Option<&super::candidate::CandidateId>,
    phase: CompilePhase,
) -> Result<(), CompileError> {
    let SnapshotBinding::Captured {
        format_version,
        memory,
        overlay,
        state,
        capture_point_version,
    } = snapshot
    else {
        return Err(CompileError::new(phase, CompileErrorKind::InvalidInput));
    };
    if format_version != super::contracts::SNAPSHOT_FORMAT_VERSION
        || capture_point_version != super::contracts::SNAPSHOT_CAPTURE_POINT_VERSION
        || memory.role != super::artifacts::ArtifactRole::MemorySnapshot
        || overlay.role != super::artifacts::ArtifactRole::OverlaySnapshot
        || state.role != super::artifacts::ArtifactRole::StateManifest
    {
        return Err(integrity(phase));
    }
    for descriptor in [memory, overlay] {
        store
            .open_verified_blob(
                &descriptor.to_store_descriptor(),
                descriptor.size,
                crate::ImportPhase::Publish,
            )
            .map_err(|error| CompileError::from_import(phase, error))?;
    }
    let mut state_file = store
        .open_verified_blob(
            &state.to_store_descriptor(),
            MAX_SNAPSHOT_STATE_BYTES,
            crate::ImportPhase::Publish,
        )
        .map_err(|error| CompileError::from_import(phase, error))?;
    let mut state_bytes = Vec::with_capacity(usize::try_from(state.size).unwrap_or(0));
    state_file
        .read_to_end(&mut state_bytes)
        .map_err(|_| CompileError::new(phase, CompileErrorKind::Io))?;
    let embedded = SnapshotManifest::decode(&state_bytes)
        .map_err(|_| integrity(phase))?
        .header()
        .candidate_id;
    let snapshot_candidate = match candidate {
        Some(candidate) => SnapshotCandidateId::new(*candidate.digest().as_bytes())
            .map_err(|_| integrity(phase))?,
        None => embedded,
    };
    let evidence = |descriptor: super::artifacts::ArtifactDescriptor| ArtifactEvidence {
        digest: Digest::from_bytes(*descriptor.digest.as_bytes()),
        size: descriptor.size,
    };
    inspect_capture(
        &state_bytes,
        evidence(memory),
        evidence(overlay),
        CaptureExpectation {
            candidate_id: snapshot_candidate,
            memory: evidence(memory),
            overlay: evidence(overlay),
            state: evidence(state),
        },
    )
    .map_err(|_| integrity(phase))?;
    Ok(())
}

const fn integrity(phase: CompilePhase) -> CompileError {
    CompileError::new(phase, CompileErrorKind::Integrity)
}

/// Publishes the ready Generation manifest for a certified Candidate, manifest last.
///
/// The token must certify this exact Candidate, so a certification obtained for other bytes
/// cannot promote these.
///
/// # Errors
///
/// Returns [`CompileErrorKind::Integrity`] when the token names another Candidate, and a
/// publication failure otherwise.
pub fn promote_candidate(
    store: &Path,
    candidate: &PublishedCandidate,
    certification: &Certification,
) -> Result<PublishedGeneration, CompileError> {
    if certification.candidate() != &candidate.id {
        return Err(CompileError::new(
            CompilePhase::Publish,
            CompileErrorKind::Integrity,
        ));
    }
    let store = Store::open(store)
        .map_err(|error| CompileError::from_import(CompilePhase::Publish, error))?;
    publish_certified(&store, &candidate.manifest, certification)
}

#[cfg(test)]
mod tests;
