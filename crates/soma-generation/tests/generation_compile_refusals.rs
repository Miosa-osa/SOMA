//! What the Generation compiler must refuse: an image for the wrong architecture, a template
//! that disagrees with the tree or the profile it names, and a host missing the pinned
//! toolchains.
//!
//! Each refusal is asserted by phase and kind together, so a rejection that happens for the
//! right reason at the wrong point in the compile is still a failure. What the compiler must
//! produce when it does not refuse lives in `generation_compile.rs`.

mod support;

use std::fs;

use soma::{MachineShape, OciImage};
use soma_generation::{
    CompileErrorKind, CompilePhase, LifetimeLimits, StartupBehavior, TemplateImage,
    TemplateRevision,
};
use support::{
    fixture_tree::AGENT,
    generation::{compile, compile_with_template},
    rootfs::{TarEntry, normalize_layers, normalize_layers_for, tar},
};

#[test]
fn non_amd64_image_is_rejected_before_any_tool_runs() {
    let (_fixture, normalized) = normalize_layers(&[tar(&[TarEntry::file(b"a", b"x")])]);
    let workload = normalized.workload();
    let error = TemplateRevision::new(
        TemplateImage::new(
            OciImage::parse("example.test/fixture:arm64").unwrap(),
            workload.manifest_digest().clone(),
            workload.platform().clone(),
        ),
        MachineShape::new(1, 256, 64).unwrap(),
        StartupBehavior::readiness_only(),
        LifetimeLimits::new(60).unwrap(),
        1,
    )
    .unwrap_err();
    assert_eq!(error.phase(), CompilePhase::ResolveInputs);
    assert_eq!(error.kind(), CompileErrorKind::Unsupported);
}

#[test]
fn template_that_disagrees_with_the_tree_or_profile_is_rejected() {
    let (fixture, normalized) =
        normalize_layers_for(&[tar(&[TarEntry::file(b"a", b"x")])], "amd64");
    let scratch = tempfile::tempdir().unwrap();
    let missing = (
        scratch.path().join("no-erofs"),
        scratch.path().join("no-e2fs"),
    );
    let workload = normalized.workload();
    let image = |digest: &str| {
        TemplateImage::new(
            OciImage::parse("example.test/fixture:amd64").unwrap(),
            soma::OciDigest::parse(digest).unwrap(),
            workload.platform().clone(),
        )
    };
    let other = image(&format!("sha256:{}", "ab".repeat(32)));
    let mismatched = TemplateRevision::new(
        other,
        MachineShape::new(1, 256, 64).unwrap(),
        StartupBehavior::readiness_only(),
        LifetimeLimits::new(60).unwrap(),
        1,
    )
    .unwrap();
    let error = compile_with_template(
        &mismatched,
        &normalized,
        &fixture.store,
        scratch.path(),
        &missing,
        AGENT,
    )
    .unwrap_err();
    assert_eq!(error.phase(), CompilePhase::ResolveInputs);
    assert_eq!(error.kind(), CompileErrorKind::Integrity);

    let unknown_class = TemplateRevision::new(
        image(workload.manifest_digest().as_str()),
        MachineShape::new(1, 256, 96).unwrap(),
        StartupBehavior::readiness_only(),
        LifetimeLimits::new(60).unwrap(),
        1,
    )
    .unwrap();
    let error = compile_with_template(
        &unknown_class,
        &normalized,
        &fixture.store,
        scratch.path(),
        &missing,
        AGENT,
    )
    .unwrap_err();
    assert_eq!(error.phase(), CompilePhase::ResolveInputs);
    assert_eq!(error.kind(), CompileErrorKind::Unsupported);
    assert!(LifetimeLimits::new(0).is_err());
    assert!(StartupBehavior::with_workload_probe(vec![0]).is_err());
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
