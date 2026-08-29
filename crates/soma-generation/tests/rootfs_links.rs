mod support;

use support::rootfs::{TarEntry, normalize_layers, read_tree, tar};

#[test]
fn symlink_targets_are_preserved_as_raw_metadata_and_never_followed() {
    let layer = tar(&[
        TarEntry::directory(b"links"),
        TarEntry::symlink(b"links/absolute", b"/outside/root"),
        TarEntry::symlink(b"links/relative", b"../../outside"),
    ]);

    let (fixture, normalized) = normalize_layers(&[layer]);
    let entries = read_tree(&fixture.store, normalized.tree_manifest_digest());

    let absolute = entries
        .iter()
        .find(|entry| entry.path == b"links/absolute")
        .unwrap();
    let relative = entries
        .iter()
        .find(|entry| entry.path == b"links/relative")
        .unwrap();
    assert_eq!(
        (absolute.kind, absolute.payload.as_slice()),
        (3, b"/outside/root".as_slice())
    );
    assert_eq!(
        (relative.kind, relative.payload.as_slice()),
        (3, b"../../outside".as_slice())
    );
}

#[test]
fn forward_hardlinks_form_one_stable_inode_group() {
    let link_first = tar(&[
        TarEntry::hardlink(b"z-alias", b"m-data"),
        TarEntry::file(b"m-data", b"same inode"),
        TarEntry::hardlink(b"a-alias", b"z-alias"),
    ]);
    let file_first = tar(&[
        TarEntry::file(b"m-data", b"same inode"),
        TarEntry::hardlink(b"a-alias", b"m-data"),
        TarEntry::hardlink(b"z-alias", b"a-alias"),
    ]);

    let (first_fixture, first) = normalize_layers(&[link_first]);
    let (_, second) = normalize_layers(&[file_first]);

    assert_eq!(first.tree_manifest_digest(), second.tree_manifest_digest());
    assert_eq!(first.logical_file_bytes(), 10);
    assert_eq!(first.content_blob_count(), 1);
    let entries = read_tree(&first_fixture.store, first.tree_manifest_digest());
    let anchor = entries
        .iter()
        .find(|entry| entry.path == b"a-alias")
        .unwrap();
    assert_eq!(anchor.kind, 2);
    for alias in [b"m-data".as_slice(), b"z-alias".as_slice()] {
        let entry = entries.iter().find(|entry| entry.path == alias).unwrap();
        assert_eq!(
            (entry.kind, entry.payload.as_slice()),
            (5, b"a-alias".as_slice())
        );
    }
}

#[test]
fn replacing_one_hardlink_path_does_not_mutate_the_other_inode() {
    let lower = tar(&[
        TarEntry::file(b"original", b"old"),
        TarEntry::hardlink(b"alias", b"original"),
    ]);
    let upper = tar(&[TarEntry::file(b"original", b"new-value")]);

    let (fixture, normalized) = normalize_layers(&[lower, upper]);
    let entries = read_tree(&fixture.store, normalized.tree_manifest_digest());

    assert_eq!(normalized.logical_file_bytes(), 12);
    assert_eq!(normalized.content_blob_count(), 2);
    assert_eq!(
        entries
            .iter()
            .find(|entry| entry.path == b"alias")
            .unwrap()
            .kind,
        2
    );
    assert_eq!(
        entries
            .iter()
            .find(|entry| entry.path == b"original")
            .unwrap()
            .kind,
        2
    );
}

#[test]
fn same_layer_hardlink_chain_never_resolves_through_a_stale_lower_target() {
    let lower = tar(&[
        TarEntry::file(b"old", b"old bytes"),
        TarEntry::file(b"z", b"stale bytes"),
    ]);
    let upper = tar(&[
        TarEntry::hardlink(b"a", b"z"),
        TarEntry::hardlink(b"z", b"old"),
    ]);

    let (fixture, normalized) = normalize_layers(&[lower, upper]);
    let entries = read_tree(&fixture.store, normalized.tree_manifest_digest());

    assert_eq!(normalized.logical_file_bytes(), 9);
    let anchor = entries.iter().find(|entry| entry.path == b"a").unwrap();
    let alias = entries.iter().find(|entry| entry.path == b"z").unwrap();
    assert_eq!(anchor.kind, 2);
    assert_eq!((alias.kind, alias.payload.as_slice()), (5, b"a".as_slice()));
}
