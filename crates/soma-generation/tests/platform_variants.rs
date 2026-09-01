//! How a descriptor's declared platform and the image config's own platform combine into the
//! one effective platform an import is allowed to claim.
//!
//! The arm64 `v8` variant is the case that exercises every rule at once: a request may leave it
//! unspecified, a descriptor may declare it, a config may refine it, and the two may disagree.
//! Each test fixes one of those and asserts either the effective platform or the refusal.

mod support;

use serde_json::json;
use soma::{OciDigest, OciPlatform, WorkloadIdentity};
use soma_generation::{
    ImportErrorKind, ImportLimits, ImportOciLayout, OciSelection, import_oci_layout,
};
use support::{CONFIG, Fixture, Image, MANIFEST, PLAIN, descriptor, digest};

#[test]
fn unspecified_requested_variant_accepts_arm64_v8_descriptor() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"plain");
    let mut selected = descriptor(MANIFEST, &image.manifest_digest, image.manifest_size);
    selected["platform"] = json!({"os": "linux", "architecture": "arm64", "variant": "v8"});
    fixture.write_index(&[selected]);

    let imported = import(
        &fixture,
        OciSelection::Platform(&OciPlatform::linux_arm64()),
    );

    let effective = OciPlatform::new("linux", "arm64", Some("v8".to_owned())).unwrap();
    assert_eq!(imported.workload().platform(), &effective);
}

#[test]
fn generic_exact_identity_preserves_effective_arm64_v8() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"plain");
    let mut selected = descriptor(MANIFEST, &image.manifest_digest, image.manifest_size);
    selected["platform"] = json!({"os": "linux", "architecture": "arm64", "variant": "v8"});
    fixture.write_index(&[selected]);
    let identity = WorkloadIdentity::new(
        OciDigest::parse(image.manifest_digest).unwrap(),
        OciPlatform::linux_arm64(),
        None,
    );

    let imported = import(&fixture, OciSelection::Exact(&identity));

    let effective = OciPlatform::new("linux", "arm64", Some("v8".to_owned())).unwrap();
    assert_eq!(imported.workload().platform(), &effective);
}

#[test]
fn concrete_exact_arm64_v8_matches_descriptor_when_config_omits_variant() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"plain");
    let mut selected = descriptor(MANIFEST, &image.manifest_digest, image.manifest_size);
    selected["platform"] = json!({"os": "linux", "architecture": "arm64", "variant": "v8"});
    fixture.write_index(&[selected]);
    let effective = OciPlatform::new("linux", "arm64", Some("v8".to_owned())).unwrap();
    let identity = WorkloadIdentity::new(
        OciDigest::parse(image.manifest_digest).unwrap(),
        effective.clone(),
        None,
    );

    let imported = import(&fixture, OciSelection::Exact(&identity));

    assert_eq!(imported.workload().platform(), &effective);
}

#[test]
fn concrete_config_variant_completes_generic_descriptor_platform() {
    let fixture = Fixture::new();
    let image = add_image_with_config_variant(&fixture, "v8");
    let mut selected = descriptor(MANIFEST, &image.manifest_digest, image.manifest_size);
    selected["platform"] = json!({"os": "linux", "architecture": "arm64"});
    fixture.write_index(&[selected]);
    let requested = OciPlatform::new("linux", "arm64", Some("v8".to_owned())).unwrap();

    let imported = import(&fixture, OciSelection::Platform(&requested));

    assert_eq!(imported.workload().platform(), &requested);
}

#[test]
fn requested_concrete_variant_must_match_effective_platform() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"plain");
    let mut selected = descriptor(MANIFEST, &image.manifest_digest, image.manifest_size);
    selected["platform"] = json!({"os": "linux", "architecture": "arm64", "variant": "v8"});
    fixture.write_index(&[selected]);
    let requested = OciPlatform::new("linux", "arm64", Some("v9".to_owned())).unwrap();
    let identity = WorkloadIdentity::new(
        OciDigest::parse(image.manifest_digest).unwrap(),
        requested,
        None,
    );

    let error = try_import(&fixture, OciSelection::Exact(&identity)).unwrap_err();

    assert_eq!(error.kind(), ImportErrorKind::Integrity);
}

#[test]
fn generic_descriptor_can_be_skipped_after_config_refines_another_variant() {
    let fixture = Fixture::new();
    let image = add_image_with_config_variant(&fixture, "v9");
    let mut selected = descriptor(MANIFEST, &image.manifest_digest, image.manifest_size);
    selected["platform"] = json!({"os": "linux", "architecture": "arm64"});
    fixture.write_index(&[selected]);
    let requested = OciPlatform::new("linux", "arm64", Some("v8".to_owned())).unwrap();
    let error = try_import(&fixture, OciSelection::Platform(&requested)).unwrap_err();
    assert_eq!(error.kind(), ImportErrorKind::NotFound);
}

#[test]
fn conflicting_concrete_descriptor_and_config_variants_fail() {
    let fixture = Fixture::new();
    let image = add_image_with_config_variant(&fixture, "v9");
    let mut selected = descriptor(MANIFEST, &image.manifest_digest, image.manifest_size);
    selected["platform"] = json!({"os": "linux", "architecture": "arm64", "variant": "v8"});
    fixture.write_index(&[selected]);

    let error = try_import(
        &fixture,
        OciSelection::Platform(&OciPlatform::linux_arm64()),
    )
    .unwrap_err();

    assert_eq!(error.kind(), ImportErrorKind::Integrity);
}

fn import(fixture: &Fixture, selection: OciSelection<'_>) -> soma_generation::ImportedOci {
    try_import(fixture, selection).unwrap()
}

fn try_import(
    fixture: &Fixture,
    selection: OciSelection<'_>,
) -> Result<soma_generation::ImportedOci, soma_generation::ImportError> {
    import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        selection,
        ImportLimits::default(),
    ))
}

fn add_image_with_config_variant(fixture: &Fixture, variant: &str) -> Image {
    let layer = support::tar_layer(b"plain");
    let layer_digest = fixture.put_blob(&layer);
    let config = serde_json::to_vec(&json!({
        "architecture": "arm64",
        "os": "linux",
        "variant": variant,
        "rootfs": {"type": "layers", "diff_ids": [digest(&layer)]},
    }))
    .unwrap();
    let config_digest = fixture.put_blob(&config);
    let manifest = serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": MANIFEST,
        "config": descriptor(CONFIG, &config_digest, config.len()),
        "layers": [descriptor(PLAIN, &layer_digest, layer.len())],
    }))
    .unwrap();
    let manifest_digest = fixture.put_blob(&manifest);
    Image {
        manifest_digest,
        manifest_size: manifest.len(),
        config_digest,
        layer_digest,
    }
}
