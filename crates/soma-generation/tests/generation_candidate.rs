//! P0.4 gates: a Candidate is never discoverable, resolvable, or promotable as a Generation.

mod support;

use std::{fs, path::Path, thread};

use soma::GenerationId;
use soma_generation::{
    CandidateId, CompileErrorKind, CompilePhase, Incompatibility, certify_candidate,
    verify_candidate, verify_generation,
};
use support::{
    fixture_tree::{AGENT, fixture_layers},
    generation::{compile, test_profile, toolchains},
    rootfs::{TarEntry, normalize_layers_for, tar},
};

/// Magic of a certified, ready Generation manifest.
const READY_MAGIC: &[u8] = b"SOMAGEN\0";
/// Magic of a Generation Candidate manifest.
const CANDIDATE_MAGIC: &[u8] = b"SOMACAN\0";

fn objects_with_magic(store: &Path, magic: &[u8]) -> Vec<String> {
    let blobs = store.join("v1/blobs/sha256");
    let Ok(entries) = fs::read_dir(&blobs) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| fs::read(entry.path()).is_ok_and(|bytes| bytes.starts_with(magic)))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

/// The `GenerationId` a party would guess from a Candidate's digest.
fn guessed_generation_id(candidate: &CandidateId) -> GenerationId {
    GenerationId::new(candidate.as_str().to_owned()).expect("a canonical sha256 identity")
}

#[test]
fn a_successful_compilation_publishes_a_candidate_and_no_ready_generation() {
    let Some(tools) = toolchains("a_successful_compilation_publishes_a_candidate") else {
        return;
    };
    let (fixture, normalized) = normalize_layers_for(&fixture_layers(), "amd64");
    let scratch = tempfile::tempdir().unwrap();
    let compiled = compile(&normalized, &fixture.store, scratch.path(), &tools, AGENT).unwrap();

    let candidates = objects_with_magic(&fixture.store, CANDIDATE_MAGIC);
    assert_eq!(candidates.len(), 1, "exactly one Candidate object");
    assert_eq!(
        candidates[0],
        compiled.id().as_str().strip_prefix("sha256:").unwrap()
    );
    assert!(
        objects_with_magic(&fixture.store, READY_MAGIC).is_empty(),
        "a ready Generation manifest exists before certification"
    );
    assert!(!compiled.candidate.launchable());

    let verified = verify_candidate(&fixture.store, compiled.id(), &test_profile()).unwrap();
    assert_eq!(verified.id, *compiled.id());
    assert_eq!(verified.manifest, compiled.candidate.manifest);
}

#[test]
fn generation_resolution_refuses_a_candidate_object() {
    let Some(tools) = toolchains("generation_resolution_refuses_a_candidate_object") else {
        return;
    };
    let (fixture, normalized) = normalize_layers_for(&fixture_layers(), "amd64");
    let scratch = tempfile::tempdir().unwrap();
    let compiled = compile(&normalized, &fixture.store, scratch.path(), &tools, AGENT).unwrap();

    // A party that learned the Candidate digest and re-labelled it as a Generation identity
    // still cannot resolve it: the stored bytes carry the Candidate magic.
    let guessed = guessed_generation_id(compiled.id());
    let error = verify_generation(&fixture.store, &guessed, &test_profile())
        .expect_err("a Candidate must never resolve as a Generation");
    assert_eq!(error.phase(), CompilePhase::EncodeManifest);
    assert_eq!(error.kind(), CompileErrorKind::InvalidInput);
}

#[test]
fn certification_is_the_only_promotion_path_and_it_is_not_implemented() {
    let Some(tools) = toolchains("certification_is_the_only_promotion_path") else {
        return;
    };
    let (fixture, normalized) = normalize_layers_for(&fixture_layers(), "amd64");
    let scratch = tempfile::tempdir().unwrap();
    let compiled = compile(&normalized, &fixture.store, scratch.path(), &tools, AGENT).unwrap();

    let error = certify_candidate(&fixture.store, &compiled.candidate, &test_profile())
        .expect_err("boot, capture, and certification have no implementation");
    assert_eq!(error.phase(), CompilePhase::Certify);
    assert_eq!(error.kind(), CompileErrorKind::Unimplemented);
    assert!(
        objects_with_magic(&fixture.store, READY_MAGIC).is_empty(),
        "a failed certification left a ready Generation identity"
    );
}

#[test]
fn every_failure_before_certification_leaves_no_manifest_of_either_kind() {
    let (fixture, normalized) =
        normalize_layers_for(&[tar(&[TarEntry::file(b"a", b"x")])], "amd64");
    let scratch = tempfile::tempdir().unwrap();
    let missing = (
        scratch.path().join("no-erofs"),
        scratch.path().join("no-e2fs"),
    );

    let error = compile(&normalized, &fixture.store, scratch.path(), &missing, AGENT).unwrap_err();

    assert_eq!(error.phase(), CompilePhase::FormatRoot);
    assert!(objects_with_magic(&fixture.store, READY_MAGIC).is_empty());
    assert!(objects_with_magic(&fixture.store, CANDIDATE_MAGIC).is_empty());
}

#[test]
fn concurrent_identical_builders_converge_on_one_candidate_object() {
    let Some(tools) = toolchains("concurrent_identical_builders_converge") else {
        return;
    };
    let layers = fixture_layers();
    let (fixture, normalized) = normalize_layers_for(&layers, "amd64");
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();

    let identities = thread::scope(|scope| {
        let left = scope.spawn(|| {
            compile(&normalized, &fixture.store, first.path(), &tools, AGENT)
                .map(|compiled| compiled.id().clone())
        });
        let right = scope.spawn(|| {
            compile(&normalized, &fixture.store, second.path(), &tools, AGENT)
                .map(|compiled| compiled.id().clone())
        });
        (left.join().unwrap(), right.join().unwrap())
    });
    let left = identities.0.expect("first builder");
    let right = identities.1.expect("second builder");

    assert_eq!(left, right, "identical builders diverged");
    assert_eq!(
        objects_with_magic(&fixture.store, CANDIDATE_MAGIC).len(),
        1,
        "concurrent identical builders published more than one Candidate object"
    );
    assert!(
        objects_with_magic(&fixture.store, READY_MAGIC).is_empty(),
        "a ready Generation appeared without certification"
    );
}

#[test]
fn a_correctly_encoded_candidate_incompatible_with_the_exact_profile_is_rejected() {
    let Some(tools) = toolchains("a_correctly_encoded_candidate_incompatible_with_the_profile")
    else {
        return;
    };
    let (fixture, normalized) = normalize_layers_for(&fixture_layers(), "amd64");
    let scratch = tempfile::tempdir().unwrap();
    let compiled = compile(&normalized, &fixture.store, scratch.path(), &tools, AGENT).unwrap();

    // Every byte is canonical and every artifact is present; only the host profile disagrees.
    let mut narrower = test_profile();
    narrower.overlay_capacities = vec![512 * 1024 * 1024];
    let error = verify_candidate(&fixture.store, compiled.id(), &narrower)
        .expect_err("an incompatible host profile must reject a well formed Candidate");
    assert_eq!(error.kind(), CompileErrorKind::Unsupported);
    assert_eq!(
        error.incompatibility(),
        Some(Incompatibility::OverlayCapacity)
    );

    let mut newer_policy = test_profile();
    newer_policy.policy_version = 1;
    assert!(verify_candidate(&fixture.store, compiled.id(), &newer_policy).is_ok());
}

#[test]
fn no_resolution_can_report_a_launchable_generation_yet() {
    let Some(tools) = toolchains("no_resolution_can_report_a_launchable_generation_yet") else {
        return;
    };
    let (fixture, normalized) = normalize_layers_for(&fixture_layers(), "amd64");
    let scratch = tempfile::tempdir().unwrap();
    let compiled = compile(&normalized, &fixture.store, scratch.path(), &tools, AGENT).unwrap();

    // The Candidate resolution has no launchability at all, and the Generation resolution
    // cannot succeed while certification and snapshot verification are unimplemented.
    assert!(verify_candidate(&fixture.store, compiled.id(), &test_profile()).is_ok());
    let guessed = guessed_generation_id(compiled.id());
    assert!(verify_generation(&fixture.store, &guessed, &test_profile()).is_err());
    assert!(!compiled.candidate.launchable());
}
