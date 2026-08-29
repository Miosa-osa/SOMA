//! The Candidate identity, its publication, and the token that promotes one to a Generation.
//!
//! A Candidate is the complete immutable output of the compiler phases that exist today.
//! It is stored under its own magic, media type, and identity type, so no Launch resolution can
//! reach it and no caller can hand it where a `GenerationId` is required.
//! Only [`Certification`] turns a Candidate into a ready Generation, and only the certification
//! gates can produce that token.

use std::fmt;

use soma::GenerationId;

use super::{
    artifacts::{ArtifactDescriptor, Sha256Digest},
    manifest::{GenerationManifest, SnapshotBinding},
};

/// Identity of one Generation Candidate.
///
/// The value is `sha256:` plus the digest of the exact Candidate manifest bytes.
/// It is a distinct type from `GenerationId` on purpose: every Launch and resolution interface
/// takes a `GenerationId`, so a Candidate cannot be substituted even by mistake.
///
/// ```compile_fail
/// use soma_generation::{CandidateId, CompilerProfile, verify_generation};
/// use std::path::Path;
///
/// let candidate = CandidateId::of(b"candidate bytes");
/// let _ = verify_generation(Path::new("/store"), &candidate, &CompilerProfile::v1());
/// ```
#[derive(Clone, Eq, PartialEq, Hash)]
pub struct CandidateId(String);

impl CandidateId {
    /// Derives the identity from exact canonical Candidate manifest bytes.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256Digest::of(bytes).to_string())
    }

    /// Returns the canonical `sha256:` identity string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(super) fn digest(&self) -> Sha256Digest {
        super::identity::digest_of(&self.0)
    }
}

impl fmt::Debug for CandidateId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("CandidateId").field(&self.0).finish()
    }
}

/// One Candidate manifest that has been published under its Candidate identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedCandidate {
    /// The identity derived from the exact Candidate manifest bytes.
    pub id: CandidateId,
    /// The Candidate manifest artifact descriptor.
    pub descriptor: ArtifactDescriptor,
    /// The decoded manifest that was published.
    pub manifest: GenerationManifest,
}

impl PublishedCandidate {
    /// Always `false`: a Candidate is not a Generation and can never be launched.
    #[must_use]
    pub const fn launchable(&self) -> bool {
        false
    }
}

/// Proof that every gate a ready Generation requires has passed for one exact Candidate.
///
/// The type has no public constructor.
/// It can only be produced by the certification gates, so a failed or revoked Candidate cannot
/// be promoted without running them again.
///
/// ```compile_fail
/// use soma_generation::{CandidateId, Certification};
///
/// let forged = Certification { candidate: CandidateId::of(b"x"), snapshot: todo!() };
/// ```
#[derive(Debug)]
pub struct Certification {
    candidate: CandidateId,
    snapshot: SnapshotBinding,
}

impl Certification {
    /// The Candidate these gates certified.
    #[must_use]
    pub const fn candidate(&self) -> &CandidateId {
        &self.candidate
    }

    pub(super) const fn snapshot(&self) -> SnapshotBinding {
        self.snapshot
    }

    /// Builds the token the certification gates would produce.
    ///
    /// Only the crate's own tests may call this, so no caller outside the gates can forge one.
    #[cfg(test)]
    pub(super) const fn for_gate_tests(candidate: CandidateId, snapshot: SnapshotBinding) -> Self {
        Self {
            candidate,
            snapshot,
        }
    }
}

/// One Generation that has been published as ready after complete certification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedGeneration {
    /// The identity derived from the exact ready manifest bytes.
    pub id: GenerationId,
    /// The ready manifest artifact descriptor.
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
