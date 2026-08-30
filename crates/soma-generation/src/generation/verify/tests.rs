//! Compatibility matrix: every manifest field gets a positive, negative, boundary, and
//! cross-field case, and every rejection names one exact invariant.

mod fields;
mod hostile;

use crate::generation::{
    artifacts::Sha256Digest, manifest::GenerationManifest, manifest::fixture,
    request::CompilerProfile,
};

use super::{Incompatibility, require_profile};

const MIB: u64 = 1024 * 1024;

fn profile() -> CompilerProfile {
    let mut profile = CompilerProfile::v1();
    profile.overlay_capacities = vec![256 * MIB, 1024 * MIB];
    profile
}

fn accepted(manifest: &GenerationManifest) {
    assert_eq!(
        require_profile(manifest, &profile()).map_err(|error| error.incompatibility()),
        Ok(()),
        "a compatible manifest was rejected"
    );
}

/// Applies one mutation and requires exactly the named invariant to reject it.
#[track_caller]
fn rejected(reason: Incompatibility, mutate: impl FnOnce(&mut GenerationManifest)) {
    let mut manifest = fixture::profile_v1();
    mutate(&mut manifest);
    let error = require_profile(&manifest, &profile()).expect_err("the mutation must be rejected");
    assert_eq!(error.incompatibility(), Some(reason));
    assert_eq!(
        error.phase(),
        crate::generation::error::CompilePhase::VerifyGeneration
    );
}

#[test]
fn the_profile_v1_fixture_is_accepted_unchanged() {
    accepted(&fixture::profile_v1());
}

#[test]
fn identity_and_source_fields_are_validated() {
    rejected(Incompatibility::PolicyVersion, |m| {
        m.compiler_policy_version = 2;
    });
    rejected(Incompatibility::SourcePlatform, |m| {
        m.source.platform = soma::OciPlatform::new("linux", "arm64", None).unwrap();
    });
    rejected(Incompatibility::SourcePlatform, |m| {
        m.source.platform =
            soma::OciPlatform::new("linux", "amd64", Some("v2".to_owned())).unwrap();
    });
    rejected(Incompatibility::ZeroDigest, |m| {
        m.source.oci_manifest_digest = Sha256Digest::from_bytes([0; 32]);
    });
    rejected(Incompatibility::ZeroDigest, |m| {
        m.tree.digest = Sha256Digest::from_bytes([0; 32]);
        m.root.uuid = crate::generation::erofs::derive_root_uuid(&m.tree.digest);
    });
}

#[test]
fn the_tree_size_bound_is_exact_at_both_edges() {
    let mut manifest = fixture::profile_v1();
    manifest.tree.size = 512 * MIB;
    accepted(&manifest);
    rejected(Incompatibility::TreeSize, |m| {
        m.tree.size = 0;
    });
    rejected(Incompatibility::TreeSize, |m| {
        m.tree.size = 512 * MIB + 1;
    });
}

#[test]
fn root_binding_fields_are_validated_against_the_tree_and_the_profile() {
    rejected(Incompatibility::RootUuid, |m| {
        m.root.uuid = [0; 16];
    });
    rejected(Incompatibility::RootFormat, |m| {
        m.root.format_profile = "erofs/other".to_owned();
    });
    rejected(Incompatibility::RootFormat, |m| {
        m.root.formatter_revision = "1.9.3".to_owned();
    });
    rejected(Incompatibility::ZeroDigest, |m| {
        m.root.formatter_digest = Sha256Digest::from_bytes([0; 32]);
    });
    rejected(Incompatibility::ZeroDigest, |m| {
        m.root.builder_environment_digest = Sha256Digest::from_bytes([0; 32]);
    });
    rejected(Incompatibility::RootSize, |m| {
        m.root.descriptor.size = 0;
    });
    rejected(Incompatibility::RootSize, |m| {
        m.root.descriptor.size = 4097;
    });
    rejected(Incompatibility::RootSize, |m| {
        m.root.descriptor.size = profile().max_root_bytes + 4096;
    });

    let mut manifest = fixture::profile_v1();
    manifest.root.descriptor.size = 4096;
    accepted(&manifest);
}

#[test]
fn overlay_capacities_bounds_and_sizes_must_agree() {
    rejected(Incompatibility::OverlayProfile, |m| {
        m.overlay.uuid_derivation_version = 2;
    });
    rejected(Incompatibility::OverlayProfile, |m| {
        m.overlay.feature_profile = "ext4/other".to_owned();
    });
    rejected(Incompatibility::OverlayCapacity, |m| {
        m.overlay.templates.clear();
    });
    rejected(Incompatibility::OverlayCapacity, |m| {
        m.overlay.templates[0].capacity = 512 * MIB;
        m.overlay.templates[0].descriptor.size = 512 * MIB;
        m.overlay.minimum_capacity = 512 * MIB;
        m.template.writable_storage_bytes = 512 * MIB;
    });
    rejected(Incompatibility::OverlaySize, |m| {
        m.overlay.templates[0].descriptor.size = 256 * MIB - 1;
    });
    rejected(Incompatibility::OverlayBounds, |m| {
        m.overlay.minimum_capacity = 1024 * MIB;
    });
    rejected(Incompatibility::OverlayBounds, |m| {
        m.overlay.maximum_capacity = 256 * MIB;
    });

    let mut manifest = fixture::profile_v1();
    manifest.overlay.templates.truncate(1);
    manifest.overlay.maximum_capacity = 256 * MIB;
    accepted(&manifest);
}
