mod support;

use support::rootfs::{TarEntry, normalize_layers as normalize, read_layer_tree, read_tree, tar};

#[test]
fn whiteout_is_applied_before_same_layer_addition_independent_of_tar_order() {
    let lower = tar(&[
        TarEntry::directory(b"app"),
        TarEntry::file(b"app/value", b"old"),
    ]);
    let whiteout_first = tar(&[
        TarEntry::file(b"app/.wh.value", b""),
        TarEntry::file(b"app/value", b"new"),
    ]);
    let addition_first = tar(&[
        TarEntry::file(b"app/value", b"new"),
        TarEntry::file(b"app/.wh.value", b""),
    ]);

    let (first_fixture, first) = normalize(&[lower.clone(), whiteout_first]);
    let (second_fixture, second) = normalize(&[lower, addition_first]);

    assert_eq!(first.tree_manifest_digest(), second.tree_manifest_digest());
    let entries = read_layer_tree(&first_fixture.store, first.tree_manifest_digest());
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.path.as_slice())
            .collect::<Vec<_>>(),
        [b"".as_slice(), b"app".as_slice(), b"app/value".as_slice()]
    );
    assert!(
        read_tree(&second_fixture.store, second.tree_manifest_digest())
            .iter()
            .all(|entry| !entry.path.windows(4).any(|window| window == b".wh."))
    );
}

#[test]
fn opaque_whiteout_removes_only_lower_children_and_keeps_new_children() {
    let lower = tar(&[
        TarEntry::directory(b"cache"),
        TarEntry::file(b"cache/old-a", b"a"),
        TarEntry::file(b"cache/old-b", b"b"),
    ]);
    let upper = tar(&[
        TarEntry::file(b"cache/new", b"new"),
        TarEntry::file(b"cache/.wh..wh..opq", b""),
    ]);

    let (fixture, normalized) = normalize(&[lower, upper]);
    let paths: Vec<_> = read_layer_tree(&fixture.store, normalized.tree_manifest_digest())
        .into_iter()
        .map(|entry| entry.path)
        .collect();

    assert_eq!(
        paths,
        [b"".to_vec(), b"cache".to_vec(), b"cache/new".to_vec()]
    );
}

#[test]
fn ordinary_whiteout_without_replacement_removes_the_lower_subtree() {
    let lower = tar(&[
        TarEntry::directory(b"removed"),
        TarEntry::file(b"removed/child", b"gone"),
        TarEntry::file(b"kept", b"here"),
    ]);
    let upper = tar(&[TarEntry::file(b".wh.removed", b"")]);

    let (fixture, normalized) = normalize(&[lower, upper]);
    let paths: Vec<_> = read_layer_tree(&fixture.store, normalized.tree_manifest_digest())
        .into_iter()
        .map(|entry| entry.path)
        .collect();

    assert_eq!(paths, [b"".to_vec(), b"kept".to_vec()]);
}

#[test]
fn whiteout_only_paths_create_their_missing_parent_directory_deterministically() {
    for marker in [
        TarEntry::file(b"created/.wh.missing", b""),
        TarEntry::file(b"created/.wh..wh..opq", b""),
    ] {
        let layer = tar(&[marker]);
        let (fixture, normalized) = normalize(&[layer]);
        let paths: Vec<_> = read_layer_tree(&fixture.store, normalized.tree_manifest_digest())
            .into_iter()
            .map(|entry| entry.path)
            .collect();
        assert_eq!(paths, [b"".to_vec(), b"created".to_vec()]);
    }
}

#[test]
fn explicit_root_directory_replaces_metadata_without_removing_children() {
    let lower = tar(&[TarEntry::file(b"kept", b"yes")]);
    let upper = tar(&[TarEntry::directory(b".")
        .mode(0o1750)
        .ownership(42, 84)
        .mtime(99)]);

    let (fixture, normalized) = normalize(&[lower, upper]);
    let entries = read_layer_tree(&fixture.store, normalized.tree_manifest_digest());

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].path, b"");
    assert_eq!(
        (
            entries[0].mode,
            entries[0].uid,
            entries[0].gid,
            entries[0].mtime
        ),
        (0o1750, 42, 84, 99)
    );
    assert_eq!(entries[1].path, b"kept");
}
