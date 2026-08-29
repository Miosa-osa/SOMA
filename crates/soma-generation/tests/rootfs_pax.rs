mod support;

use soma::OciPlatform;
use soma_generation::{
    ImportErrorKind, ImportLimits, ImportOciLayout, ImportedOci, NormalizeErrorKind,
    NormalizeOciRootfs, NormalizePhase, OciSelection, RootfsLimits, import_oci_layout,
    normalize_oci_rootfs,
};
use support::{
    Fixture,
    rootfs::{
        TarEntry, add_layers, global_pax_layer, local_pax_layer, malformed_local_pax_layer,
        normalize_layers, tar,
    },
};

#[test]
fn local_pax_path_preserves_effective_bytes_and_tree_identity() {
    let effective = "certs/NetLock_Főtanúsítvány.pem".as_bytes();
    let pax = local_pax_layer(
        &TarEntry::file(b"placeholder", b"certificate"),
        &[("path", effective)],
    );
    let direct = tar(&[TarEntry::file(effective, b"certificate")]);

    let (_, from_pax) = normalize_layers(&[pax]);
    let (_, from_header) = normalize_layers(&[direct]);

    assert_eq!(
        from_pax.tree_manifest_digest(),
        from_header.tree_manifest_digest()
    );
}

#[test]
fn local_pax_linkpath_preserves_effective_bytes_and_tree_identity() {
    let effective = "../certs/NetLock_Főtanúsítvány.pem".as_bytes();
    let pax = local_pax_layer(
        &TarEntry::symlink(b"certs/current", b"placeholder"),
        &[("linkpath", effective)],
    );
    let direct = tar(&[TarEntry::symlink(b"certs/current", effective)]);

    let (_, from_pax) = normalize_layers(&[pax]);
    let (_, from_header) = normalize_layers(&[direct]);

    assert_eq!(
        from_pax.tree_manifest_digest(),
        from_header.tree_manifest_digest()
    );
}

#[test]
fn unknown_and_duplicate_local_keys_fail_closed() {
    for (layer, expected) in [
        (
            local_pax_layer(&TarEntry::file(b"value", b"body"), &[("mtime", b"1.5")]),
            NormalizeErrorKind::Unsupported,
        ),
        (
            local_pax_layer(
                &TarEntry::file(b"value", b"body"),
                &[("path", b"first"), ("path", b"second")],
            ),
            NormalizeErrorKind::InvalidInput,
        ),
        (
            local_pax_layer(
                &TarEntry::symlink(b"value", b"target"),
                &[("linkpath", b"first"), ("linkpath", b"second")],
            ),
            NormalizeErrorKind::InvalidInput,
        ),
        (
            local_pax_layer(&TarEntry::file(b"value", b"body"), &[("path", b"bad-\xff")]),
            NormalizeErrorKind::InvalidInput,
        ),
    ] {
        let (fixture, imported) = import_layers(&layer).unwrap();
        let error = normalize_oci_rootfs(NormalizeOciRootfs::new(
            &imported,
            &fixture.store,
            RootfsLimits::default(),
        ))
        .unwrap_err();
        assert_eq!(error.kind(), expected);
    }
}

#[test]
fn local_linkpath_on_a_non_link_entry_is_rejected() {
    let layer = local_pax_layer(
        &TarEntry::file(b"value", b"body"),
        &[("linkpath", b"target")],
    );
    let (fixture, imported) = import_layers(&layer).unwrap();
    let error = normalize_oci_rootfs(NormalizeOciRootfs::new(
        &imported,
        &fixture.store,
        RootfsLimits::default(),
    ))
    .unwrap_err();

    assert_eq!(error.kind(), NormalizeErrorKind::InvalidInput);
}

#[test]
fn local_pax_is_bounded_by_the_normalization_metadata_budget() {
    let layer = local_pax_layer(
        &TarEntry::file(b"placeholder", b"body"),
        &[("path", b"effective")],
    );
    let (fixture, imported) = import_layers(&layer).unwrap();
    let limits = RootfsLimits {
        max_metadata_bytes: 1,
        ..RootfsLimits::default()
    };
    let error = normalize_oci_rootfs(NormalizeOciRootfs::new(&imported, &fixture.store, limits))
        .unwrap_err();

    assert_eq!(error.phase(), NormalizePhase::VerifyLayer);
    assert_eq!(error.kind(), NormalizeErrorKind::LimitExceeded);
}

#[test]
fn local_pax_byte_budget_is_shared_across_selected_layers() {
    let layers = [
        local_pax_layer(&TarEntry::file(b"placeholder", b"one"), &[("path", b"a")]),
        local_pax_layer(&TarEntry::file(b"placeholder", b"two"), &[("path", b"b")]),
    ];
    let per_layer = local_pax_body_size(&layers[0]);
    assert_eq!(local_pax_body_size(&layers[1]), per_layer);
    let fixture = Fixture::new();
    let image = add_layers(&fixture, &layers);
    fixture.write_direct_index(&image, true);
    let platform = OciPlatform::linux_arm64();
    let imported = import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Platform(&platform),
        ImportLimits::default(),
    ))
    .unwrap();
    let limits = RootfsLimits {
        max_metadata_bytes: per_layer,
        ..RootfsLimits::default()
    };

    let error = normalize_oci_rootfs(NormalizeOciRootfs::new(&imported, &fixture.store, limits))
        .unwrap_err();
    assert_eq!(error.phase(), NormalizePhase::VerifyLayer);
    assert_eq!(error.kind(), NormalizeErrorKind::LimitExceeded);
}

#[test]
fn global_pax_fails_during_import_and_malformed_local_pax_fails_normalization() {
    let global = import_layers(&global_pax_layer())
        .err()
        .expect("global PAX must fail");
    assert_eq!(global.kind(), ImportErrorKind::Unsupported);

    let (fixture, imported) =
        import_layers(&malformed_local_pax_layer(b"not-a-pax-record\n")).unwrap();
    let malformed = normalize_oci_rootfs(NormalizeOciRootfs::new(
        &imported,
        &fixture.store,
        RootfsLimits::default(),
    ))
    .unwrap_err();
    assert_eq!(malformed.kind(), NormalizeErrorKind::InvalidInput);
}

fn import_layers(layer: &[u8]) -> Result<(Fixture, ImportedOci), soma_generation::ImportError> {
    let fixture = Fixture::new();
    let image = add_layers(&fixture, &[layer.to_vec()]);
    fixture.write_direct_index(&image, true);
    let platform = OciPlatform::linux_arm64();
    let imported = import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Platform(&platform),
        ImportLimits::default(),
    ))?;
    Ok((fixture, imported))
}

fn local_pax_body_size(layer: &[u8]) -> u64 {
    let mut archive = tar::Archive::new(layer);
    archive
        .entries()
        .unwrap()
        .raw(true)
        .map(|entry| entry.unwrap())
        .find(|entry| entry.header().entry_type().is_pax_local_extensions())
        .unwrap()
        .size()
}
