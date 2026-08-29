mod support;

use soma::OciPlatform;
use soma_generation::{
    ImportErrorKind, ImportLimits, ImportOciLayout, ImportPhase, ImportedOci, NormalizeErrorKind,
    NormalizeOciRootfs, OciSelection, RootfsLimits, import_oci_layout, normalize_oci_rootfs,
};
use support::{
    Fixture,
    rootfs::{TarEntry, add_layers, pax_layer, sparse_layer, tar},
};
use tar::EntryType;

#[test]
fn malformed_reserved_whiteout_entries_fail_closed() {
    let malformed = [
        tar(&[TarEntry::symlink(b".wh.victim", b"target")]),
        tar(&[TarEntry::file(b".wh.victim", b"not empty")]),
        tar(&[TarEntry::file(b".wh.", b"")]),
        tar(&[TarEntry::file(b".wh...", b"")]),
    ];

    for layer in malformed {
        let (fixture, imported) = import_layers(&[layer]);
        assert_kind(
            normalize_oci_rootfs(NormalizeOciRootfs::new(
                &imported,
                &fixture.store,
                RootfsLimits::default(),
            )),
            NormalizeErrorKind::InvalidInput,
        );
    }
}

#[test]
fn reserved_whiteout_names_are_forbidden_in_every_non_marker_component() {
    for layer in [
        tar(&[TarEntry::file(b".wh.victim/child", b"hidden")]),
        tar(&[TarEntry::file(b"parent/.wh.victim/child", b"hidden")]),
        tar(&[TarEntry::file(b".wh.parent/.wh.victim", b"")]),
    ] {
        let (fixture, imported) = import_layers(&[layer]);
        assert_kind(
            normalize_oci_rootfs(NormalizeOciRootfs::new(
                &imported,
                &fixture.store,
                RootfsLimits::default(),
            )),
            NormalizeErrorKind::InvalidInput,
        );
    }
}

#[test]
fn device_contiguous_and_unknown_nodes_are_rejected() {
    for kind in [
        EntryType::Char,
        EntryType::Block,
        EntryType::Continuous,
        EntryType::new(b's'),
    ] {
        let layer = tar(&[TarEntry::special(b"unsupported", kind)]);
        let (fixture, imported) = import_layers(&[layer]);
        assert_kind(
            normalize_oci_rootfs(NormalizeOciRootfs::new(
                &imported,
                &fixture.store,
                RootfsLimits::default(),
            )),
            NormalizeErrorKind::Unsupported,
        );
    }
}

#[test]
fn empty_path_root_replacement_and_symlink_parent_conflicts_are_rejected() {
    for layer in [
        tar(&[TarEntry::file(b".", b"not-root-directory")]),
        tar(&[
            TarEntry::symlink(b"link", b"outside"),
            TarEntry::file(b"link/child", b"must not follow"),
        ]),
        tar(&[
            TarEntry::file(b"regular", b"parent"),
            TarEntry::file(b"regular/child", b"conflict"),
        ]),
    ] {
        let (fixture, imported) = import_layers(&[layer]);
        assert_kind(
            normalize_oci_rootfs(NormalizeOciRootfs::new(
                &imported,
                &fixture.store,
                RootfsLimits::default(),
            )),
            NormalizeErrorKind::InvalidInput,
        );
    }
}

#[test]
fn ownership_and_mode_values_outside_the_v1_profile_are_rejected() {
    for layer in [
        tar(&[TarEntry::file(b"uid", b"body").ownership(u64::from(u32::MAX) + 1, 0)]),
        tar(&[TarEntry::file(b"gid", b"body").ownership(0, u64::from(u32::MAX) + 1)]),
        tar(&[TarEntry::file(b"mode", b"body").mode(0o10_000)]),
    ] {
        let (fixture, imported) = import_layers(&[layer]);
        assert_kind(
            normalize_oci_rootfs(NormalizeOciRootfs::new(
                &imported,
                &fixture.store,
                RootfsLimits::default(),
            )),
            NormalizeErrorKind::Unsupported,
        );
    }
}

#[test]
fn empty_archive_name_is_rejected_before_normalization() {
    let fixture = Fixture::new();
    let layer = tar(&[TarEntry::file(b"", b"empty")]);
    let image = add_layers(&fixture, &[layer]);
    fixture.write_direct_index(&image, true);
    let result = import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Platform(&OciPlatform::linux_arm64()),
        ImportLimits::default(),
    ));

    assert_eq!(result.unwrap_err().kind(), ImportErrorKind::InvalidInput);
}

#[test]
fn unresolved_cyclic_and_directory_hardlinks_are_rejected() {
    for layer in [
        tar(&[TarEntry::hardlink(b"alias", b"missing")]),
        tar(&[
            TarEntry::hardlink(b"first", b"second"),
            TarEntry::hardlink(b"second", b"first"),
        ]),
        tar(&[
            TarEntry::directory(b"directory"),
            TarEntry::hardlink(b"alias", b"directory"),
        ]),
    ] {
        let (fixture, imported) = import_layers(&[layer]);
        assert_kind(
            normalize_oci_rootfs(NormalizeOciRootfs::new(
                &imported,
                &fixture.store,
                RootfsLimits::default(),
            )),
            NormalizeErrorKind::InvalidInput,
        );
    }
}

#[test]
fn hardlink_cannot_replace_an_ancestor_of_its_own_target() {
    let lower = tar(&[
        TarEntry::directory(b"tree"),
        TarEntry::file(b"tree/target", b"content"),
    ]);
    let upper = tar(&[TarEntry::hardlink(b"tree", b"tree/target")]);
    let (fixture, imported) = import_layers(&[lower, upper]);

    assert_kind(
        normalize_oci_rootfs(NormalizeOciRootfs::new(
            &imported,
            &fixture.store,
            RootfsLimits::default(),
        )),
        NormalizeErrorKind::InvalidInput,
    );
}

#[test]
fn gnu_sparse_layer_fails_during_import() {
    let fixture = Fixture::new();
    let image = add_layers(&fixture, &[sparse_layer()]);
    fixture.write_direct_index(&image, true);
    let platform = OciPlatform::linux_arm64();
    let error = import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Platform(&platform),
        ImportLimits::default(),
    ))
    .unwrap_err();

    assert_eq!(error.phase(), ImportPhase::VerifyLayer);
    assert_eq!(error.kind(), ImportErrorKind::Unsupported);
}

#[test]
fn pax_metadata_and_xattrs_are_rejected_instead_of_silently_lost() {
    for layer in [
        pax_layer("value", "mtime", b"1.5"),
        pax_layer("value", "SCHILY.xattr.security.capability", b"secret"),
        pax_layer("value", "unknown.vendor.key", b"value"),
    ] {
        let (fixture, imported) = import_layers(&[layer]);
        assert_kind(
            normalize_oci_rootfs(NormalizeOciRootfs::new(
                &imported,
                &fixture.store,
                RootfsLimits::default(),
            )),
            NormalizeErrorKind::Unsupported,
        );
    }
}

fn import_layers(layers: &[Vec<u8>]) -> (Fixture, ImportedOci) {
    let fixture = Fixture::new();
    let image = add_layers(&fixture, layers);
    fixture.write_direct_index(&image, true);
    let platform = OciPlatform::linux_arm64();
    let imported = import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Platform(&platform),
        ImportLimits::default(),
    ))
    .unwrap();
    (fixture, imported)
}

fn assert_kind<T: std::fmt::Debug>(
    result: Result<T, soma_generation::NormalizeError>,
    expected: NormalizeErrorKind,
) {
    assert_eq!(result.unwrap_err().kind(), expected);
}
