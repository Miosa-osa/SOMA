mod support;

use std::fs;

use serde_json::json;
use soma::{GenerationId, OciDigest, OciPlatform, WorkloadIdentity};
use soma_generation::{
    ImportErrorKind, ImportLimits, ImportOciLayout, OciSelection, import_oci_layout,
};
use support::{CONFIG, Fixture, INDEX, Image, MANIFEST, PLAIN, descriptor, digest, tar_layer};

#[test]
fn unknown_auxiliary_descriptor_is_skipped() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    let mut selected = descriptor(MANIFEST, &image.manifest_digest, image.manifest_size);
    selected["platform"] = json!({"os": "linux", "architecture": "arm64"});
    fixture.write_index(&[
        json!({"mediaType": "application/vnd.example.aux", "digest": "not-a-digest", "size": -1}),
        selected,
    ]);

    assert!(run(&fixture, ImportLimits::default()).is_ok());
}

#[test]
fn negative_known_descriptor_size_is_rejected_before_conversion() {
    let fixture = Fixture::new();
    fixture.write_index(&[json!({
        "mediaType": MANIFEST,
        "digest": format!("sha256:{}", "a".repeat(64)),
        "size": -1,
        "platform": {"os": "linux", "architecture": "arm64"},
    })]);

    assert_kind(
        run(&fixture, ImportLimits::default()),
        ImportErrorKind::InvalidInput,
    );
}

#[test]
fn descriptor_size_and_digest_mismatches_fail_integrity() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    let mut selected = descriptor(MANIFEST, &image.manifest_digest, image.manifest_size + 1);
    selected["platform"] = json!({"os": "linux", "architecture": "arm64"});
    fixture.write_index(&[selected]);
    assert_kind(
        run(&fixture, ImportLimits::default()),
        ImportErrorKind::Integrity,
    );

    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    fs::write(fixture.blob_path(&image.manifest_digest), b"wrong").unwrap();
    fixture.write_direct_index(&image, true);
    assert_kind(
        run(&fixture, ImportLimits::default()),
        ImportErrorKind::Integrity,
    );
}

#[test]
fn verified_config_must_match_descriptor_platform() {
    let fixture = Fixture::new();
    let layer = tar_layer(b"layer");
    let layer_digest = fixture.put_blob(&layer);
    let config = serde_json::to_vec(&json!({
        "architecture": "amd64",
        "os": "linux",
        "rootfs": {"type": "layers", "diff_ids": [digest(&layer)]},
    }))
    .unwrap();
    let image = raw_image(&fixture, &config, &layer_digest, layer.len());
    fixture.write_direct_index(&image, true);

    assert_kind(
        run(&fixture, ImportLimits::default()),
        ImportErrorKind::Integrity,
    );
}

#[test]
fn ordered_layer_count_must_equal_diff_id_count() {
    let fixture = Fixture::new();
    let layer = tar_layer(b"layer");
    let layer_digest = fixture.put_blob(&layer);
    let config = serde_json::to_vec(&json!({
        "architecture": "arm64", "os": "linux",
        "rootfs": {"type": "layers", "diff_ids": []},
    }))
    .unwrap();
    let image = raw_image(&fixture, &config, &layer_digest, layer.len());
    fixture.write_direct_index(&image, true);

    assert_kind(
        run(&fixture, ImportLimits::default()),
        ImportErrorKind::Integrity,
    );
}

#[test]
fn expanded_layer_digest_must_equal_ordered_diff_id() {
    let fixture = Fixture::new();
    let layer = tar_layer(b"layer");
    let layer_digest = fixture.put_blob(&layer);
    let config = serde_json::to_vec(&json!({
        "architecture": "arm64", "os": "linux",
        "rootfs": {"type": "layers", "diff_ids": [digest(b"different")]},
    }))
    .unwrap();
    let image = raw_image(&fixture, &config, &layer_digest, layer.len());
    fixture.write_direct_index(&image, true);

    assert_kind(
        run(&fixture, ImportLimits::default()),
        ImportErrorKind::Integrity,
    );
}

#[test]
fn malformed_plain_layer_fails_closed() {
    let fixture = Fixture::new();
    let layer = b"not a tar stream";
    let layer_digest = fixture.put_blob(layer);
    let config = serde_json::to_vec(&json!({
        "architecture": "arm64", "os": "linux",
        "rootfs": {"type": "layers", "diff_ids": [digest(layer)]},
    }))
    .unwrap();
    let image = raw_image(&fixture, &config, &layer_digest, layer.len());
    fixture.write_direct_index(&image, true);

    assert_kind(
        run(&fixture, ImportLimits::default()),
        ImportErrorKind::Integrity,
    );
}

#[test]
fn expanded_and_total_blob_limits_fail_closed() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"12345");
    fixture.write_direct_index(&image, true);
    let expanded = ImportLimits {
        max_expanded_bytes: 4,
        ..ImportLimits::default()
    };
    assert_kind(run(&fixture, expanded), ImportErrorKind::LimitExceeded);

    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    fixture.write_direct_index(&image, true);
    let total = ImportLimits {
        max_total_blob_bytes: 1,
        ..ImportLimits::default()
    };
    assert_kind(run(&fixture, total), ImportErrorKind::LimitExceeded);
    assert!(!fixture.store.join("v1").exists());
}

#[test]
fn two_matching_manifests_are_ambiguous() {
    let fixture = Fixture::new();
    let first = fixture.add_plain_image(b"first");
    let second = fixture.add_plain_image(b"second");
    let mut first_descriptor = descriptor(MANIFEST, &first.manifest_digest, first.manifest_size);
    let mut second_descriptor = descriptor(MANIFEST, &second.manifest_digest, second.manifest_size);
    first_descriptor["platform"] = json!({"os": "linux", "architecture": "arm64"});
    second_descriptor["platform"] = json!({"os": "linux", "architecture": "arm64"});
    fixture.write_index(&[first_descriptor, second_descriptor]);

    assert_kind(
        run(&fixture, ImportLimits::default()),
        ImportErrorKind::Ambiguous,
    );
}

#[test]
fn descriptor_budget_includes_manifest_config_and_layers() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    fixture.write_direct_index(&image, true);
    let limits = ImportLimits {
        max_descriptors: 2,
        ..ImportLimits::default()
    };

    assert_kind(run(&fixture, limits), ImportErrorKind::LimitExceeded);
}

#[test]
fn nested_index_depth_is_bounded() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    let mut current = descriptor(MANIFEST, &image.manifest_digest, image.manifest_size);
    for _ in 0..9 {
        let bytes = serde_json::to_vec(&json!({
            "schemaVersion": 2, "mediaType": INDEX, "manifests": [current],
        }))
        .unwrap();
        let nested_digest = fixture.put_blob(&bytes);
        current = descriptor(INDEX, &nested_digest, bytes.len());
    }
    fixture.write_index(&[current]);

    assert_kind(
        run(&fixture, ImportLimits::default()),
        ImportErrorKind::LimitExceeded,
    );
}

#[test]
fn conflicting_duplicate_manifest_descriptors_fail_integrity() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    let mut first = descriptor(MANIFEST, &image.manifest_digest, image.manifest_size);
    let mut conflicting = descriptor(MANIFEST, &image.manifest_digest, image.manifest_size + 1);
    first["platform"] = json!({"os": "linux", "architecture": "arm64"});
    conflicting["platform"] = json!({"os": "linux", "architecture": "arm64"});
    fixture.write_index(&[first, conflicting]);

    assert_kind(
        run(&fixture, ImportLimits::default()),
        ImportErrorKind::Integrity,
    );
}

#[test]
fn paths_are_redacted_and_generation_identity_is_not_accepted() {
    let fixture = Fixture::new();
    fixture.write_index(&[]);
    let platform = OciPlatform::linux_arm64();
    let request = ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Platform(&platform),
        ImportLimits::default(),
    );
    let request_debug = format!("{request:?}");
    assert!(!request_debug.contains(fixture.layout.to_str().unwrap()));
    let error = import_oci_layout(request).unwrap_err();
    assert!(!format!("{error:?} {error}").contains(fixture.layout.to_str().unwrap()));

    let identity = WorkloadIdentity::new(
        OciDigest::parse(format!("sha256:{}", "a".repeat(64))).unwrap(),
        OciPlatform::linux_arm64(),
        Some(GenerationId::new(format!("sha256:{}", "b".repeat(64))).unwrap()),
    );
    let result = import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Exact(&identity),
        ImportLimits::default(),
    ));
    assert_kind(result, ImportErrorKind::InvalidInput);
}

fn raw_image(fixture: &Fixture, config: &[u8], layer_digest: &str, layer_size: usize) -> Image {
    let config_digest = fixture.put_blob(config);
    let manifest = serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": MANIFEST,
        "config": descriptor(CONFIG, &config_digest, config.len()),
        "layers": [descriptor(PLAIN, layer_digest, layer_size)],
    }))
    .unwrap();
    let manifest_digest = fixture.put_blob(&manifest);
    Image {
        manifest_digest,
        manifest_size: manifest.len(),
        config_digest,
        layer_digest: layer_digest.to_owned(),
    }
}

fn run(
    fixture: &Fixture,
    limits: ImportLimits,
) -> Result<soma_generation::ImportedOci, soma_generation::ImportError> {
    import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Platform(&OciPlatform::linux_arm64()),
        limits,
    ))
}

fn assert_kind<T: std::fmt::Debug>(
    result: Result<T, soma_generation::ImportError>,
    expected: ImportErrorKind,
) {
    assert_eq!(result.unwrap_err().kind(), expected);
}
