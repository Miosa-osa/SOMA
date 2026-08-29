mod support;

use std::fs;

use soma::OciPlatform;
use soma_generation::{
    ImportLimits, ImportOciLayout, NormalizeOciRootfs, OciSelection, RootfsLimits,
    import_oci_layout, normalize_oci_rootfs,
};
use support::{Fixture, digest};

#[test]
fn imported_layer_normalizes_to_an_immutable_content_addressed_tree() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"hello rootfs");
    fixture.write_direct_index(&image, true);
    let platform = OciPlatform::linux_arm64();
    let imported = import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Platform(&platform),
        ImportLimits::default(),
    ))
    .unwrap();

    let normalized = normalize_oci_rootfs(NormalizeOciRootfs::new(
        &imported,
        &fixture.store,
        RootfsLimits::default(),
    ))
    .unwrap();

    assert_eq!(normalized.workload(), imported.workload());
    assert_eq!(
        normalized.source_import_manifest_digest(),
        imported.import_manifest_digest()
    );
    assert_eq!(normalized.entry_count(), 3);
    assert_eq!(normalized.logical_file_bytes(), 12);
    assert_eq!(normalized.content_blob_count(), 1);
    assert_eq!(normalized.content_blob_bytes(), 12);
    let manifest = fixture
        .store
        .join("v1/blobs/sha256")
        .join(&normalized.tree_manifest_digest().as_str()[7..]);
    assert_eq!(
        fs::metadata(manifest).unwrap().len(),
        normalized.tree_manifest_size()
    );
    let content = fixture
        .store
        .join("v1/blobs/sha256")
        .join(&digest(b"hello rootfs")[7..]);
    assert_eq!(fs::read(&content).unwrap(), b"hello rootfs");
    assert!(fs::metadata(content).unwrap().permissions().readonly());
}
