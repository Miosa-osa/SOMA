mod support;

use support::rootfs::{TarEntry, normalize_layers, read_tree, tar};

#[test]
fn metadata_fifo_non_utf8_path_and_implicit_parent_are_preserved_portably() {
    let layer = tar(&[
        TarEntry::file(b"implicit/\xffpayload", b"bytes")
            .mode(0o6754)
            .ownership(1234, 5678)
            .mtime(9_876),
        TarEntry::fifo(b"events")
            .mode(0o1640)
            .ownership(7, 8)
            .mtime(9),
    ]);

    let (fixture, normalized) = normalize_layers(&[layer]);
    let entries = read_tree(&fixture.store, normalized.tree_manifest_digest());

    let implicit = entries
        .iter()
        .find(|entry| entry.path == b"implicit")
        .unwrap();
    assert_eq!(
        (
            implicit.kind,
            implicit.mode,
            implicit.uid,
            implicit.gid,
            implicit.mtime
        ),
        (1, 0o755, 0, 0, 0)
    );
    let file = entries
        .iter()
        .find(|entry| entry.path == b"implicit/\xffpayload")
        .unwrap();
    assert_eq!(
        (file.kind, file.mode, file.uid, file.gid, file.mtime),
        (2, 0o6754, 1234, 5678, 9_876)
    );
    let fifo = entries
        .iter()
        .find(|entry| entry.path == b"events")
        .unwrap();
    assert_eq!(
        (fifo.kind, fifo.mode, fifo.uid, fifo.gid, fifo.mtime),
        (4, 0o1640, 7, 8, 9)
    );
}

#[test]
fn directory_metadata_merge_and_both_type_replacements_have_oci_semantics() {
    let lower = tar(&[
        TarEntry::directory(b"kept-dir").mode(0o700),
        TarEntry::file(b"kept-dir/child", b"kept"),
        TarEntry::directory(b"becomes-file"),
        TarEntry::file(b"becomes-file/removed", b"gone"),
        TarEntry::file(b"becomes-dir", b"gone"),
    ]);
    let upper = tar(&[
        TarEntry::directory(b"kept-dir")
            .mode(0o755)
            .ownership(10, 11),
        TarEntry::file(b"becomes-file", b"now-file"),
        TarEntry::directory(b"becomes-dir").mode(0o711),
        TarEntry::file(b"becomes-dir/new", b"new"),
    ]);

    let (fixture, normalized) = normalize_layers(&[lower, upper]);
    let entries = read_tree(&fixture.store, normalized.tree_manifest_digest());

    assert!(entries.iter().any(|entry| entry.path == b"kept-dir/child"));
    let kept = entries
        .iter()
        .find(|entry| entry.path == b"kept-dir")
        .unwrap();
    assert_eq!((kept.mode, kept.uid, kept.gid), (0o755, 10, 11));
    assert_eq!(
        entries
            .iter()
            .find(|entry| entry.path == b"becomes-file")
            .unwrap()
            .kind,
        2
    );
    assert!(
        !entries
            .iter()
            .any(|entry| entry.path == b"becomes-file/removed")
    );
    assert_eq!(
        entries
            .iter()
            .find(|entry| entry.path == b"becomes-dir")
            .unwrap()
            .kind,
        1
    );
    assert!(entries.iter().any(|entry| entry.path == b"becomes-dir/new"));
}
