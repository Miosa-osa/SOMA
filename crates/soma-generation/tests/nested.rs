//! Traversal of an OCI layout that nests one index inside another, and the layer work that
//! follows once a leaf manifest has been chosen.
//!
//! The rules that decide which platform a descriptor and its config together name live in
//! `platform_variants.rs`; these are about reaching a leaf and reading it.

mod support;

use std::io::Write as _;

use flate2::{Compression, write::GzEncoder};
use serde_json::json;
use soma::{OciDigest, OciPlatform, WorkloadIdentity};
use soma_generation::{
    ImportErrorKind, ImportLimits, ImportOciLayout, OciSelection, import_oci_layout,
};
use support::{Fixture, GZIP, INDEX, MANIFEST, descriptor};

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
