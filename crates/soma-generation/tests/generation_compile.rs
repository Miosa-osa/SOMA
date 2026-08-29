mod support;

use std::{fs, path::Path, process::Command, thread, time::Duration};

use soma_generation::{
    ArtifactRole, CompileErrorKind, CompilePhase, CompiledGeneration, SnapshotBinding,
    erofs::format_uuid, verify_generation,
};
use support::{
    generation::{compile, test_profile, toolchains},
    rootfs::{TarEntry, local_pax_layer, normalize_layers, normalize_layers_for, tar},
};

const LONG_NAME: &[u8] =
    b"long/component-name-that-exceeds-the-one-hundred-byte-ustar-field-and-forces-a-pax-path-record-0123456789";
const AGENT: &[u8] = b"synthetic-guest-agent";

fn big_body() -> Vec<u8> {
    (0..10_240_u32).map(|value| (value % 251) as u8).collect()
}

fn fixture_layers() -> Vec<Vec<u8>> {
    let big = big_body();
    let exact = vec![0xab_u8; 4096];
    let first = tar(&[
        TarEntry::directory(b"etc")
            .mode(0o750)
            .ownership(5, 7)
            .mtime(1_500_000_000),
        TarEntry::file(b"etc/a", b"alpha").mtime(1_400_000_000),
        TarEntry::hardlink(b"etc/a-hard", b"etc/a"),
        TarEntry::file(b"etc/z", b"zulu")
            .mode(0o4755)
            .ownership(3_000_000, 80_000),
        TarEntry::symlink(b"a-link", b"../etc/a"),
        TarEntry::fifo(b"pipe").mode(0o600),
        TarEntry::directory(b"tmp").mode(0o1777),
        TarEntry::file(b"big", &big),
        TarEntry::file(b"exact", &exact),
        TarEntry::file(b"empty", b""),
    ]);
    let second = tar(&[
        TarEntry::directory(b"usr"),
        TarEntry::directory(b"usr/bin"),
        TarEntry::file(b"usr/bin/x", b"#!/bin/sh\n").mode(0o755),
        TarEntry::hardlink(b"usr/bin/x-hard", b"usr/bin/x"),
        TarEntry::directory(b"long"),
    ]);
    let third = local_pax_layer(&TarEntry::file(b"long/x", b"pax"), &[("path", LONG_NAME)]);
    vec![first, second, third]
}

fn digests(compiled: &CompiledGeneration) -> Vec<(ArtifactRole, String, u64)> {
    compiled
        .published
        .manifest
        .descriptors()
        .iter()
        .map(|descriptor| {
            (
                descriptor.role,
                descriptor.digest.to_string(),
                descriptor.size,
            )
        })
        .collect()
}

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
    assert_eq!(compiled_a.published.manifest, compiled_b.published.manifest);

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
        compiled_a.published.manifest.source.oci_manifest_digest,
        compiled_c.published.manifest.source.oci_manifest_digest
    );
    let mut same_source = compiled_c.published.manifest.clone();
    same_source.source = compiled_a.published.manifest.source.clone();
    assert_eq!(same_source, compiled_a.published.manifest);
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
        compiled_a.published.manifest.root.uuid
    );
    assert!(!compiled_a.published.launchable());
    assert_eq!(
        compiled_a.published.manifest.snapshot,
        SnapshotBinding::Absent
    );
    assert_eq!(compiled_a.overlay.classes.len(), 2);
    for class in &compiled_a.overlay.classes {
        assert_eq!(class.check.exit_code, Some(0));
    }
    eprintln!(
        "generation_id={} tree={} root_uuid={} artifacts={:?}",
        compiled_a.id().as_str(),
        normalized_a.tree_manifest_digest().as_str(),
        format_uuid(&compiled_a.erofs.uuid),
        digests(&compiled_a)
    );

    let verified = verify_generation(&fixture_a.store, compiled_a.id(), &test_profile()).unwrap();
    assert!(!verified.launchable);
    assert_eq!(verified.artifacts_verified, 6);
    assert_eq!(verified.manifest, compiled_a.published.manifest);
    extraction_oracle(&tools.0, &fixture_a.store, &compiled_a);
}

fn extraction_oracle(erofs_tools: &Path, store: &Path, compiled: &CompiledGeneration) {
    let root = &compiled.published.manifest.root.descriptor;
    let image = store
        .join("v1/blobs/sha256")
        .join(&root.digest.to_string()[7..]);
    let extract = tempfile::tempdir().unwrap();
    let target = extract.path().join("tree");
    let status = Command::new(erofs_tools.join("fsck.erofs"))
        .arg(format!("--extract={}", target.display()))
        .arg("--preserve-perms")
        .arg(&image)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(fs::read(target.join("etc/a")).unwrap(), b"alpha");
    assert_eq!(fs::read(target.join("etc/a-hard")).unwrap(), b"alpha");
    assert_eq!(fs::read(target.join("big")).unwrap(), big_body());
    assert_eq!(fs::read(target.join("exact")).unwrap(), vec![0xab_u8; 4096]);
    assert_eq!(fs::read(target.join("empty")).unwrap(), b"");
    assert_eq!(
        fs::read(target.join(std::str::from_utf8(LONG_NAME).unwrap())).unwrap(),
        b"pax"
    );
    assert_eq!(
        fs::read_link(target.join("a-link")).unwrap(),
        Path::new("../etc/a")
    );
    let pipe = fs::symlink_metadata(target.join("pipe")).unwrap();
    assert!(std::os::unix::fs::FileTypeExt::is_fifo(&pipe.file_type()));
    let count = walk_count(&target);
    assert_eq!(count, u64::from(compiled.erofs.entries_verified) - 1);
}

fn walk_count(path: &Path) -> u64 {
    let mut count = 0;
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        count += 1;
        if entry.file_type().unwrap().is_dir() {
            count += walk_count(&entry.path());
        }
    }
    count
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
        other_agent.published.manifest.root.descriptor,
        baseline.published.manifest.root.descriptor
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
        changed.published.manifest.root.descriptor.digest,
        baseline.published.manifest.root.descriptor.digest
    );
    assert_eq!(
        changed.published.manifest.overlay.templates,
        baseline.published.manifest.overlay.templates
    );
}

#[test]
fn tampered_artifact_fails_cross_artifact_verification() {
    let Some(tools) = toolchains("tampered_artifact_fails_cross_artifact_verification") else {
        return;
    };
    let (fixture, normalized) = normalize_layers_for(&fixture_layers(), "amd64");
    let scratch = tempfile::tempdir().unwrap();
    let compiled = compile(&normalized, &fixture.store, scratch.path(), &tools, AGENT).unwrap();
    let blob = fixture.store.join("v1/blobs/sha256").join(
        &compiled
            .published
            .manifest
            .initramfs
            .descriptor
            .digest
            .to_string()[7..],
    );
    let mut permissions = fs::metadata(&blob).unwrap().permissions();
    permissions.set_readonly(false);
    fs::set_permissions(&blob, permissions).unwrap();
    let mut bytes = fs::read(&blob).unwrap();
    bytes[200] ^= 0xff;
    fs::write(&blob, bytes).unwrap();
    let error = verify_generation(&fixture.store, compiled.id(), &test_profile()).unwrap_err();
    assert_eq!(error.phase(), CompilePhase::VerifyGeneration);
    assert!(matches!(
        error.kind(),
        CompileErrorKind::Integrity | CompileErrorKind::StoreConflict
    ));
}

#[test]
fn non_amd64_tree_is_rejected_before_any_tool_runs() {
    let (fixture, normalized) = normalize_layers(&[tar(&[TarEntry::file(b"a", b"x")])]);
    let scratch = tempfile::tempdir().unwrap();
    let missing = (
        scratch.path().join("no-erofs"),
        scratch.path().join("no-e2fs"),
    );
    let error = compile(&normalized, &fixture.store, scratch.path(), &missing, AGENT).unwrap_err();
    assert_eq!(error.phase(), CompilePhase::ResolveInputs);
    assert_eq!(error.kind(), CompileErrorKind::Unsupported);
}

#[test]
fn missing_toolchain_fails_closed_after_input_verification() {
    let (fixture, normalized) =
        normalize_layers_for(&[tar(&[TarEntry::file(b"a", b"x")])], "amd64");
    let scratch = tempfile::tempdir().unwrap();
    let missing = (
        scratch.path().join("no-erofs"),
        scratch.path().join("no-e2fs"),
    );
    let error = compile(&normalized, &fixture.store, scratch.path(), &missing, AGENT).unwrap_err();
    assert_eq!(error.phase(), CompilePhase::FormatRoot);
    assert_eq!(error.kind(), CompileErrorKind::Toolchain);
    assert!(
        !fixture
            .store
            .join("v1/blobs/sha256")
            .read_dir()
            .unwrap()
            .any(|entry| {
                let path = entry.unwrap().path();
                fs::read(path).unwrap().starts_with(b"SOMAGEN\0")
            })
    );
}
