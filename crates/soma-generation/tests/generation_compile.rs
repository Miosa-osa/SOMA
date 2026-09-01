//! What the Generation compiler must produce: the same tree compiling to byte identical
//! artifacts under the same identity, a changed input changing that identity, and a tampered
//! artifact failing cross-artifact verification.
//!
//! What it must refuse lives in `generation_compile_refusals.rs`. Both need the pinned
//! toolchains; these three cannot run without them, and skip when they are absent.

mod support;

use std::{fs, thread, time::Duration};

use soma_generation::{
    CompileErrorKind, CompilePhase, SnapshotBinding, erofs::format_uuid, verify_candidate,
};
#[cfg(unix)]
use support::fixture_tree::extraction_oracle;

use support::{
    fixture_tree::{AGENT, digests, fixture_layers},
    generation::{compile, test_profile, toolchains},
    rootfs::{TarEntry, normalize_layers_for, tar},
};

#[test]
fn same_tree_compiles_to_byte_identical_artifacts_and_identity() {
    let Some(tools) = toolchains("same_tree_compiles_to_byte_identical_artifacts_and_identity")
    else {
        return;
    };
    let layers = fixture_layers();
    let (fixture_a, normalized_a) = normalize_layers_for(&layers, "amd64");
    let scratch_a = tempfile::tempdir().unwrap();
    let compiled_a = compile(
        &normalized_a,
        &fixture_a.store,
        scratch_a.path(),
        &tools,
        AGENT,
    )
    .unwrap();

    thread::sleep(Duration::from_millis(1_100));
    let (fixture_b, normalized_b) = normalize_layers_for(&layers, "amd64");
    let scratch_b = tempfile::tempdir().unwrap();
    let compiled_b = compile(
        &normalized_b,
        &fixture_b.store,
        scratch_b.path(),
        &tools,
        AGENT,
    )
    .unwrap();
    assert_eq!(digests(&compiled_a), digests(&compiled_b));
    assert_eq!(compiled_a.id(), compiled_b.id());
    assert_eq!(compiled_a.candidate.manifest, compiled_b.candidate.manifest);

    let reversed: Vec<Vec<u8>> = layers.iter().rev().cloned().collect();
    let (fixture_c, normalized_c) = normalize_layers_for(&reversed, "amd64");
    let scratch_c = tempfile::tempdir().unwrap();
    let compiled_c = compile(
        &normalized_c,
        &fixture_c.store,
        scratch_c.path(),
        &tools,
        AGENT,
    )
    .unwrap();
    assert_eq!(
        normalized_a.tree_manifest_digest(),
        normalized_c.tree_manifest_digest()
    );
    assert_eq!(digests(&compiled_a), digests(&compiled_c));
    assert_ne!(
        compiled_a.candidate.manifest.source.oci_manifest_digest,
        compiled_c.candidate.manifest.source.oci_manifest_digest
    );
    let mut same_source = compiled_c.candidate.manifest.clone();
    same_source.source = compiled_a.candidate.manifest.source.clone();
    assert_eq!(same_source, compiled_a.candidate.manifest);
    assert_ne!(compiled_a.id(), compiled_c.id());

    assert_eq!(compiled_a.erofs.formatter_revision, "1.9.4");
    assert_eq!(compiled_a.erofs.format.exit_code, Some(0));
    assert_eq!(compiled_a.erofs.check.exit_code, Some(0));
    assert_eq!(
        compiled_a.erofs.entries_verified,
        normalized_a.entry_count()
    );
    assert_eq!(
        compiled_a.erofs.uuid,
        compiled_a.candidate.manifest.root.uuid
    );
    assert!(!compiled_a.candidate.launchable());
    assert_eq!(
        compiled_a.candidate.manifest.snapshot,
        SnapshotBinding::Absent
    );
    let overlay_a = compiled_a.overlay.as_ref().expect("overlay evidence");
    assert_eq!(overlay_a.classes.len(), 2);
    for class in &overlay_a.classes {
        assert_eq!(class.check.exit_code, Some(0));
    }
    eprintln!(
        "generation_id={} tree={} root_uuid={} artifacts={:?}",
        compiled_a.id().as_str(),
        normalized_a.tree_manifest_digest().as_str(),
        format_uuid(&compiled_a.erofs.uuid),
        digests(&compiled_a)
    );

    let verified = verify_candidate(&fixture_a.store, compiled_a.id(), &test_profile()).unwrap();
    assert_eq!(verified.artifacts_verified, 6);
    assert_eq!(verified.manifest, compiled_a.candidate.manifest);
    assert!(!compiled_a.candidate.launchable());
    #[cfg(unix)]
    extraction_oracle(&tools.0, &fixture_a.store, &compiled_a);
}

#[test]
fn changing_bound_inputs_changes_identity_while_unchanged_artifacts_stay_equal() {
    let Some(tools) = toolchains("changing_bound_inputs_changes_identity") else {
        return;
    };
    let mut layers = fixture_layers();
    let (fixture, normalized) = normalize_layers_for(&layers, "amd64");
    let scratch = tempfile::tempdir().unwrap();
    let baseline = compile(&normalized, &fixture.store, scratch.path(), &tools, AGENT).unwrap();

    let other_agent = compile(
        &normalized,
        &fixture.store,
        scratch.path(),
        &tools,
        b"other",
    )
    .unwrap();
    assert_ne!(other_agent.id(), baseline.id());
    assert_eq!(
        other_agent.candidate.manifest.root.descriptor,
        baseline.candidate.manifest.root.descriptor
    );

    layers.push(tar(&[TarEntry::file(b"etc/extra", b"more")]));
    let (fixture_extra, normalized_extra) = normalize_layers_for(&layers, "amd64");
    let scratch_extra = tempfile::tempdir().unwrap();
    let changed = compile(
        &normalized_extra,
        &fixture_extra.store,
        scratch_extra.path(),
        &tools,
        AGENT,
    )
    .unwrap();
    assert_ne!(changed.id(), baseline.id());
    assert_ne!(
        changed.candidate.manifest.root.descriptor.digest,
        baseline.candidate.manifest.root.descriptor.digest
    );
    assert_eq!(
        changed.candidate.manifest.overlay.templates,
        baseline.candidate.manifest.overlay.templates
    );
}

#[test]
#[cfg_attr(windows, allow(clippy::permissions_set_readonly_false))]
fn tampered_artifact_fails_cross_artifact_verification() {
    let Some(tools) = toolchains("tampered_artifact_fails_cross_artifact_verification") else {
        return;
    };
    let (fixture, normalized) = normalize_layers_for(&fixture_layers(), "amd64");
    let scratch = tempfile::tempdir().unwrap();
    let compiled = compile(&normalized, &fixture.store, scratch.path(), &tools, AGENT).unwrap();
    let blob = fixture.store.join("v1/blobs/sha256").join(
        &compiled
            .candidate
            .manifest
            .initramfs
            .descriptor
            .digest
            .to_string()[7..],
    );
    let mut permissions = fs::metadata(&blob).unwrap().permissions();
    #[cfg(unix)]
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o600);
    #[cfg(windows)]
    permissions.set_readonly(false);
    fs::set_permissions(&blob, permissions).unwrap();
    let mut bytes = fs::read(&blob).unwrap();
    bytes[200] ^= 0xff;
    fs::write(&blob, bytes).unwrap();
    let error = verify_candidate(&fixture.store, compiled.id(), &test_profile()).unwrap_err();
    assert_eq!(error.phase(), CompilePhase::VerifyGeneration);
    assert!(matches!(
        error.kind(),
        CompileErrorKind::Integrity | CompileErrorKind::StoreConflict
    ));
}
