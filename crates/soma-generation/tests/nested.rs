mod support;

use std::io::Write as _;

use flate2::{Compression, write::GzEncoder};
use serde_json::json;
use soma::{OciDigest, OciPlatform, WorkloadIdentity};
use soma_generation::{
    ImportErrorKind, ImportLimits, ImportOciLayout, OciSelection, import_oci_layout,
};
use support::{CONFIG, Fixture, GZIP, INDEX, Image, MANIFEST, PLAIN, descriptor, digest};

#[test]
fn nested_index_annotation_order_is_provenance_not_import_identity() {
    let first = Fixture::new();
    let image = first.add_plain_image(b"layer");
    let first_nested = first.write_nested_index(&image, true);
    let imported_first = import(&first, OciSelection::Platform(&OciPlatform::linux_arm64()));

    let second = Fixture::new();
    let second_image = second.add_plain_image(b"layer");
    let second_nested = second.write_nested_index(&second_image, false);
    let imported_second = import(&second, OciSelection::Platform(&OciPlatform::linux_arm64()));

    assert_ne!(first_nested, second_nested);
    assert_eq!(
        imported_first.import_manifest_digest(),
        imported_second.import_manifest_digest()
    );
    assert_ne!(
        imported_first.traversed_indexes(),
        imported_second.traversed_indexes()
    );
}

#[test]
fn descriptor_without_platform_is_selected_from_verified_config() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"plain");
    fixture.write_direct_index(&image, false);

    let imported = import(
        &fixture,
        OciSelection::Platform(&OciPlatform::linux_arm64()),
    );

    assert_eq!(imported.workload().platform(), &OciPlatform::linux_arm64());
}

#[test]
fn exact_selection_retains_caller_registry_index_identity() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"plain");
    fixture.write_direct_index(&image, false);
    let registry_index = OciDigest::parse(format!("sha256:{}", "a".repeat(64))).unwrap();
    let identity = WorkloadIdentity::new(
        OciDigest::parse(image.manifest_digest).unwrap(),
        OciPlatform::linux_arm64(),
        None,
    )
    .with_index_digest(registry_index.clone());

    let imported = import(&fixture, OciSelection::Exact(&identity));

    assert_eq!(imported.workload().index_digest(), Some(&registry_index));
}

#[test]
fn gzip_layer_is_bound_to_its_expanded_diff_id() {
    let expanded = support::tar_layer(b"expanded deterministic filesystem bytes");
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&expanded).unwrap();
    let compressed = encoder.finish().unwrap();
    let fixture = Fixture::new();
    let image = fixture.add_image(&compressed, &expanded, GZIP);
    fixture.write_direct_index(&image, true);

    let imported = import(
        &fixture,
        OciSelection::Platform(&OciPlatform::linux_arm64()),
    );

    assert_eq!(
        imported.workload().manifest_digest().as_str(),
        image.manifest_digest
    );
}

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

#[test]
fn nested_platform_conflict_is_not_hidden_by_leaf_selection_pruning() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"plain");
    let mut leaf = descriptor(MANIFEST, &image.manifest_digest, image.manifest_size);
    leaf["platform"] = json!({"os": "windows", "architecture": "arm64"});
    let nested = serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": INDEX,
        "manifests": [leaf],
    }))
    .unwrap();
    let nested_digest = fixture.put_blob(&nested);
    let mut selected = descriptor(INDEX, &nested_digest, nested.len());
    selected["platform"] = json!({"os": "linux", "architecture": "arm64"});
    fixture.write_index(&[selected]);

    let error = try_import(
        &fixture,
        OciSelection::Platform(&OciPlatform::linux_arm64()),
    )
    .unwrap_err();

    assert_eq!(error.kind(), ImportErrorKind::Integrity);
}

#[test]
fn gzip_expansion_obeys_the_aggregate_limit() {
    let expanded = support::tar_layer(&vec![b'x'; 32 * 1024]);
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&expanded).unwrap();
    let compressed = encoder.finish().unwrap();
    let fixture = Fixture::new();
    let image = fixture.add_image(&compressed, &expanded, GZIP);
    fixture.write_direct_index(&image, true);
    let limits = ImportLimits {
        max_expanded_bytes: 1_024,
        ..ImportLimits::default()
    };

    let result = import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Platform(&OciPlatform::linux_arm64()),
        limits,
    ));

    assert_eq!(
        result.unwrap_err().kind(),
        soma_generation::ImportErrorKind::LimitExceeded
    );
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
