//! The gate between a Candidate and a ready Generation.
//!
//! A Candidate becomes a Generation only after boot and capture, compatibility, security, and
//! certification all succeed.
//! Phases 4 and 5 of the compiler design have no implementation, so [`certify_candidate`] fails
//! closed and no ready Generation identity can exist yet.
//! [`promote_candidate`] is the only publication path for a ready manifest and it requires the
//! [`Certification`] token, so a revoked or failed Candidate cannot be promoted without running
//! the gates again.

use std::path::Path;

use super::{
    candidate::{Certification, PublishedCandidate, PublishedGeneration},
    error::{CompileError, CompileErrorKind, CompilePhase},
    publish::publish_certified,
    request::CompilerProfile,
    verify::verify_candidate,
};
use crate::store::Store;

/// Runs the boot, capture, compatibility, security, and certification gates for one Candidate.
///
/// # Errors
///
/// Returns [`CompileErrorKind::Unimplemented`] until phases 4 and 5 exist.
/// Every other failure returns the phase that rejected the Candidate.
pub fn certify_candidate(
    store: &Path,
    candidate: &PublishedCandidate,
    profile: &CompilerProfile,
) -> Result<Certification, CompileError> {
    // Compatibility and cross-artifact verification are the gates that do exist; running them
    // first means a Candidate that cannot even re-verify never reaches the unimplemented ones.
    let verified = verify_candidate(store, &candidate.id, profile)?;
    if verified.manifest != candidate.manifest {
        return Err(CompileError::new(
            CompilePhase::VerifyGeneration,
            CompileErrorKind::Integrity,
        ));
    }
    Err(CompileError::new(
        CompilePhase::Certify,
        CompileErrorKind::Unimplemented,
    ))
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
