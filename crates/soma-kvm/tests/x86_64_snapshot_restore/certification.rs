//! Hardware proof for the complete Candidate-to-ready-Generation transition.

use std::fs::File;

use soma_generation::{
    ArtifactDescriptor, ArtifactRole, CompilerProfile, PublishedCandidate, Sha256Digest,
    SnapshotSource, certify_candidate, generation_manifest, install_snapshot, promote_candidate,
    verify_generation,
};

use crate::{x86_64_sandbox_boot_host::require_kvm, x86_64_snapshot_restore_fixture as fixture};

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn captured_candidate_certifies_promotes_and_reverifies() {
    require_kvm();
    let fixture = fixture::shared();
    let descriptor = |role, digest: soma_kvm::snapshot::Digest, size| ArtifactDescriptor {
        role,
        digest: Sha256Digest::from_bytes(*digest.as_bytes()),
        size,
    };
    let mut memory = File::open(fixture.paths.memory()).unwrap();
    let mut overlay = File::open(fixture.paths.overlay()).unwrap();
    let mut state = File::open(fixture.paths.state()).unwrap();
    let snapshot = install_snapshot(
        &fixture.compiled.store,
        SnapshotSource::new(
            &mut memory,
            descriptor(
                ArtifactRole::MemorySnapshot,
                fixture.capture.memory_digest,
                fixture.capture.memory_bytes,
            ),
        ),
        SnapshotSource::new(
            &mut overlay,
            descriptor(
                ArtifactRole::OverlaySnapshot,
                fixture.capture.overlay_digest,
                fixture.capture.overlay_bytes,
            ),
        ),
        SnapshotSource::new(
            &mut state,
            descriptor(
                ArtifactRole::StateManifest,
                fixture.capture.state_digest,
                fixture.capture.state_bytes,
            ),
        ),
    )
    .expect("install captured artifacts");
    let bytes = generation_manifest::encode_candidate(fixture.compiled.manifest()).unwrap();
    let candidate = PublishedCandidate {
        id: fixture.compiled.id().clone(),
        descriptor: ArtifactDescriptor {
            role: ArtifactRole::GenerationCandidate,
            digest: Sha256Digest::of(&bytes),
            size: u64::try_from(bytes.len()).unwrap(),
        },
        manifest: fixture.compiled.manifest().clone(),
    };
    let mut profile = CompilerProfile::v1();
    profile.overlay_capacities = vec![fixture::STORAGE_MIB * 1024 * 1024];
    let certification = certify_candidate(&fixture.compiled.store, &candidate, &profile, snapshot)
        .expect("certify the captured Candidate");
    let published = promote_candidate(&fixture.compiled.store, &candidate, &certification)
        .expect("publish the ready Generation");
    let verified = verify_generation(&fixture.compiled.store, &published.id, &profile)
        .expect("reverify the ready Generation");
    assert!(published.launchable());
    assert!(verified.launchable);
    assert_eq!(verified.id, published.id);
}
