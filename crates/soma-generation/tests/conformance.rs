//! Conformance of the OCI layout reader with the image specification: which media types are
//! accepted, and which platform requirements are refused rather than quietly ignored.
//!
//! The bounds the same reader enforces on a hostile layout live in `import_bounds.rs`; these are
//! about agreeing with the specification, those are about surviving a layout that lies about size.

mod support;

use std::fs;

use serde_json::json;
use soma::{OciDigest, OciPlatform, WorkloadIdentity};
use soma_generation::{
    ImportError, ImportErrorKind, ImportLimits, ImportOciLayout, ImportPhase, OciSelection,
    import_oci_layout,
};
use support::{CONFIG, Fixture, INDEX, Image, MANIFEST, PLAIN, descriptor, digest};

#[test]
fn top_index_rejects_wrong_declared_media_type() {
    for invalid in [json!("application/vnd.example.wrong"), json!(null)] {
        let fixture = Fixture::new();
        let image = fixture.add_plain_image(b"layer");
        fixture.write_direct_index(&image, true);
        let path = fixture.layout.join("index.json");
        let mut index: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        index["mediaType"] = invalid;
        fs::write(path, serde_json::to_vec(&index).unwrap()).unwrap();
        let error = run(&fixture, ImportLimits::default()).unwrap_err();
        assert_eq!(error.phase(), ImportPhase::SelectManifest);
        assert!(matches!(
            error.kind(),
            ImportErrorKind::InvalidInput | ImportErrorKind::Unsupported
        ));
    }
}

#[test]
fn indexes_accept_an_absent_media_type() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    let nested = nested_index(&fixture, &image, None, json!({}));
    fixture.write_index(&[nested]);
    remove_field(&fixture.layout.join("index.json"), "mediaType");
    assert!(run(&fixture, ImportLimits::default()).is_ok());
}

#[test]
fn nested_index_rejects_wrong_declared_media_type() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    let nested = nested_index(
        &fixture,
        &image,
        Some("application/vnd.example.wrong"),
        json!({}),
    );
    fixture.write_index(&[nested]);

    let error = run(&fixture, ImportLimits::default()).unwrap_err();

    assert_eq!(error.phase(), ImportPhase::SelectManifest);
    assert_eq!(error.kind(), ImportErrorKind::Unsupported);
}

#[test]
fn nested_index_platform_variant_refines_an_exact_effective_leaf() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    let nested = nested_index(
        &fixture,
        &image,
        Some(INDEX),
        json!({"os": "linux", "architecture": "arm64", "variant": "v8"}),
    );
    fixture.write_index(&[nested]);
    let expected = OciPlatform::new("linux", "arm64", Some("v8".to_owned())).unwrap();
    let identity = WorkloadIdentity::new(
        OciDigest::parse(&image.manifest_digest).unwrap(),
        expected.clone(),
        None,
    );

    let imported = import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Exact(&identity),
        ImportLimits::default(),
    ))
    .unwrap();

    assert_eq!(imported.workload().platform(), &expected);
}

#[test]
fn nested_index_platform_must_agree_with_the_effective_leaf() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    let nested = nested_index(
        &fixture,
        &image,
        Some(INDEX),
        json!({"os": "windows", "architecture": "arm64"}),
    );
    fixture.write_index(&[nested]);

    let error = run(&fixture, ImportLimits::default()).unwrap_err();

    assert_eq!(error.phase(), ImportPhase::SelectManifest);
    assert_eq!(error.kind(), ImportErrorKind::Integrity);
}

#[test]
fn descriptor_platform_requirements_are_not_silently_discarded() {
    for requirements in [
        json!({"os.features": ["feature-a"]}),
        json!({"os.version": "10.0.1"}),
    ] {
        let fixture = Fixture::new();
        let image = fixture.add_plain_image(b"layer");
        let mut selected = descriptor(MANIFEST, &image.manifest_digest, image.manifest_size);
        selected["platform"] = platform_with(&requirements);
        fixture.write_index(&[selected]);

        let error = run(&fixture, ImportLimits::default()).unwrap_err();
        assert_eq!(error.phase(), ImportPhase::SelectManifest);
        assert_eq!(error.kind(), ImportErrorKind::Unsupported);
    }
}

#[test]
fn config_platform_requirements_are_not_silently_discarded() {
    for requirements in [
        json!({"os.features": ["feature-a"]}),
        json!({"os.version": "10.0.1"}),
    ] {
        let fixture = Fixture::new();
        let image = image_with_config_requirements(&fixture, &requirements);
        fixture.write_direct_index(&image, true);

        let error = run(&fixture, ImportLimits::default()).unwrap_err();
        assert_eq!(error.phase(), ImportPhase::VerifyConfig);
        assert_eq!(error.kind(), ImportErrorKind::Unsupported);
    }
}

#[test]
fn empty_os_features_do_not_create_an_unsupported_requirement() {
    let fixture = Fixture::new();
    let image = image_with_config_requirements(&fixture, &json!({"os.features": []}));
    let nested = nested_index(
        &fixture,
        &image,
        Some(INDEX),
        json!({"os": "linux", "architecture": "arm64", "os.features": []}),
    );
    fixture.write_index(&[nested]);

    assert!(run(&fixture, ImportLimits::default()).is_ok());
}

fn run(
    fixture: &Fixture,
    limits: ImportLimits,
) -> Result<soma_generation::ImportedOci, ImportError> {
    import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Platform(&OciPlatform::linux_arm64()),
        limits,
    ))
}

fn nested_index(
    fixture: &Fixture,
    image: &Image,
    media_type: Option<&str>,
    platform: serde_json::Value,
) -> serde_json::Value {
    let mut nested = json!({
        "schemaVersion": 2,
        "manifests": [{
            "mediaType": MANIFEST,
            "digest": image.manifest_digest,
            "size": image.manifest_size,
            "platform": {"os": "linux", "architecture": "arm64"},
        }],
    });
    if let Some(media_type) = media_type {
        nested["mediaType"] = json!(media_type);
    }
    let bytes = serde_json::to_vec(&nested).unwrap();
    let digest = fixture.put_blob(&bytes);
    let mut descriptor = descriptor(INDEX, &digest, bytes.len());
    if platform.as_object().is_some_and(|value| !value.is_empty()) {
        descriptor["platform"] = platform;
    }
    descriptor
}

fn platform_with(requirements: &serde_json::Value) -> serde_json::Value {
    let mut platform = json!({"os": "linux", "architecture": "arm64"});
    platform
        .as_object_mut()
        .unwrap()
        .extend(requirements.as_object().unwrap().clone());
    platform
}

fn image_with_config_requirements(fixture: &Fixture, requirements: &serde_json::Value) -> Image {
    let layer = support::tar_layer(b"layer");
    let layer_digest = fixture.put_blob(&layer);
    let mut config = json!({
        "architecture": "arm64",
        "os": "linux",
        "rootfs": {"type": "layers", "diff_ids": [digest(&layer)]},
    });
    config
        .as_object_mut()
        .unwrap()
        .extend(requirements.as_object().unwrap().clone());
    let config = serde_json::to_vec(&config).unwrap();
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

fn remove_field(path: &std::path::Path, field: &str) {
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    value.as_object_mut().unwrap().remove(field);
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}
