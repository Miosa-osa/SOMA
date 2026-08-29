mod support;

use support::rootfs::{TarEntry, normalize_layers, tar};

#[test]
fn tree_identity_excludes_layer_partition_tar_order_and_import_provenance() {
    let direct = tar(&[
        TarEntry::file(b"tree/value", b"final"),
        TarEntry::directory(b"tree"),
    ]);
    let lower = tar(&[
        TarEntry::directory(b"tree"),
        TarEntry::file(b"tree/value", b"old"),
    ]);
    let upper = tar(&[
        TarEntry::file(b"tree/value", b"final"),
        TarEntry::file(b"tree/.wh.value", b""),
    ]);

    let (_, first) = normalize_layers(&[direct]);
    let (_, second) = normalize_layers(&[lower, upper]);

    assert_ne!(
        first.source_import_manifest_digest(),
        second.source_import_manifest_digest()
    );
    assert_eq!(first.tree_manifest_digest(), second.tree_manifest_digest());
    assert_eq!(first.tree_manifest_size(), second.tree_manifest_size());
}

#[test]
fn changing_file_metadata_or_content_changes_tree_identity() {
    let base = tar(&[TarEntry::file(b"value", b"same").mode(0o644)]);
    let metadata = tar(&[TarEntry::file(b"value", b"same").mode(0o600)]);
    let content = tar(&[TarEntry::file(b"value", b"different").mode(0o644)]);

    let (_, base) = normalize_layers(&[base]);
    let (_, metadata) = normalize_layers(&[metadata]);
    let (_, content) = normalize_layers(&[content]);

    assert_ne!(base.tree_manifest_digest(), metadata.tree_manifest_digest());
    assert_ne!(base.tree_manifest_digest(), content.tree_manifest_digest());
}
