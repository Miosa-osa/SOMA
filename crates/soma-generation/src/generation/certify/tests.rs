//! Promotion gates: only a token for these exact bytes can turn a Candidate into a Generation.

use std::path::Path;

use super::{Certification, PublishedCandidate, promote_candidate};
use crate::generation::{
    artifacts::{ArtifactDescriptor, ArtifactRole, Sha256Digest},
    candidate::CandidateId,
    error::{CompileErrorKind, CompilePhase},
    manifest::{encode_candidate, fixture},
};

fn candidate() -> PublishedCandidate {
    let manifest = fixture::profile_v1();
    let bytes = encode_candidate(&manifest).expect("candidate bytes");
    PublishedCandidate {
        id: CandidateId::of(&bytes),
        descriptor: ArtifactDescriptor {
            role: ArtifactRole::GenerationCandidate,
            digest: Sha256Digest::of(&bytes),
            size: u64::try_from(bytes.len()).expect("bounded manifest"),
        },
        manifest,
    }
}

#[test]
fn a_candidate_is_never_launchable_and_names_its_own_identity() {
    let candidate = candidate();

    assert!(!candidate.launchable());
    assert!(candidate.id.as_str().starts_with("sha256:"));
    assert_eq!(candidate.descriptor.role, ArtifactRole::GenerationCandidate);
    assert_eq!(
        candidate.descriptor.media_type(),
        "application/vnd.soma.generation-candidate.v1"
    );
}

#[test]
fn a_token_for_other_bytes_cannot_promote_this_candidate() {
    let candidate = candidate();
    let other = CandidateId::of(b"another candidate");
    let certification = Certification::for_gate_tests(other, fixture::captured_snapshot());

    let error = promote_candidate(Path::new("/soma/no/such/store"), &candidate, &certification)
        .expect_err("a foreign token must not promote");

    assert_eq!(error.phase(), CompilePhase::Publish);
    assert_eq!(error.kind(), CompileErrorKind::Integrity);
}

#[test]
fn a_certification_names_the_exact_candidate_it_certified() {
    let candidate = candidate();
    let certification =
        Certification::for_gate_tests(candidate.id.clone(), fixture::captured_snapshot());

    assert_eq!(certification.candidate(), &candidate.id);
}
