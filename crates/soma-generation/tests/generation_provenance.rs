//! Every tool that materially shapes a Generation is bound by the bytes that executed.

mod support;

use std::fs;

use soma_generation::{BoundTool, BuilderEnvironment, CompilePhase, Sha256Digest};
use support::{
    fixture_tree::{AGENT, fixture_layers},
    generation::{compile, toolchains},
    rootfs::normalize_layers_for,
};

/// The complete set of external tools one profile v1 build executes, in canonical order.
const MATERIAL_TOOLS: [&str; 6] = [
    "debugfs",
    "dumpe2fs",
    "e2fsck",
    "fsck.erofs",
    "mke2fs",
    "mkfs.erofs",
];

#[test]
fn every_tool_that_shaped_an_artifact_is_bound_by_the_bytes_that_ran() {
    let Some(tools) = toolchains("every_tool_that_shaped_an_artifact_is_bound") else {
        return;
    };
    let (fixture, normalized) = normalize_layers_for(&fixture_layers(), "amd64");
    let scratch = tempfile::tempdir().unwrap();

    let compiled = compile(&normalized, &fixture.store, scratch.path(), &tools, AGENT).unwrap();

    let mut environment = compiled.erofs.tools.clone();
    environment
        .absorb(
            &compiled.overlay.as_ref().expect("overlay evidence").tools,
            CompilePhase::EncodeManifest,
        )
        .unwrap();
    let bound: Vec<&str> = environment.tools().iter().map(BoundTool::name).collect();
    assert_eq!(bound, MATERIAL_TOOLS);

    for tool in environment.tools() {
        let directory = if tools.0.join(tool.name()).is_file() {
            &tools.0
        } else {
            &tools.1
        };
        let bytes = fs::read(directory.join(tool.name())).unwrap();
        assert_eq!(
            tool.digest(),
            Sha256Digest::of(&bytes),
            "{} was bound to bytes other than the executable that ran",
            tool.name()
        );
        assert!(
            !tool.revision().is_empty(),
            "{} has no revision",
            tool.name()
        );
    }
}

#[test]
fn the_manifest_binds_the_sealed_builder_environment_rather_than_one_formatter() {
    let Some(tools) = toolchains("the_manifest_binds_the_sealed_builder_environment") else {
        return;
    };
    let (fixture, normalized) = normalize_layers_for(&fixture_layers(), "amd64");
    let scratch = tempfile::tempdir().unwrap();

    let compiled = compile(&normalized, &fixture.store, scratch.path(), &tools, AGENT).unwrap();

    let mut environment = compiled.erofs.tools.clone();
    environment
        .absorb(
            &compiled.overlay.as_ref().expect("overlay evidence").tools,
            CompilePhase::EncodeManifest,
        )
        .unwrap();
    let root = &compiled.candidate.manifest.root;
    let sealed = environment.digest(CompilePhase::EncodeManifest).unwrap();

    assert_eq!(root.builder_environment_digest, sealed);
    assert_ne!(
        root.builder_environment_digest,
        Sha256Digest::from_bytes([0; 32])
    );
    assert_ne!(
        root.builder_environment_digest, root.formatter_digest,
        "the sealed environment must cover more than the root formatter"
    );

    // Dropping any one tool from the seal changes the digest the manifest carries, so a build
    // that ran a different checker or inspector cannot present this Generation identity.
    for name in MATERIAL_TOOLS {
        let mut reduced = BuilderEnvironment::new();
        for tool in environment.tools() {
            if tool.name() != name {
                reduced
                    .bind(tool.clone(), CompilePhase::EncodeManifest)
                    .unwrap();
            }
        }
        assert_ne!(
            reduced.digest(CompilePhase::EncodeManifest).unwrap(),
            sealed,
            "dropping {name} left the builder environment digest unchanged"
        );
    }
}
