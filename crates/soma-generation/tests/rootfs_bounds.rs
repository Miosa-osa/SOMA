mod support;

use soma::OciPlatform;
use soma_generation::{
    ImportLimits, ImportOciLayout, ImportedOci, NormalizeErrorKind, NormalizeOciRootfs,
    OciSelection, RootfsLimits, import_oci_layout, normalize_oci_rootfs,
};
use support::{
    Fixture,
    rootfs::{TarEntry, add_layers, local_pax_layer, tar},
};

#[test]
fn stored_blob_expansion_and_entry_budgets_are_enforced_before_application() {
    let layer = tar(&[
        TarEntry::file(b"first", b"1"),
        TarEntry::file(b"second", b"2"),
    ]);
    for limits in [
        RootfsLimits {
            max_blob_bytes: 1,
            ..RootfsLimits::default()
        },
        RootfsLimits {
            max_expanded_bytes: 1,
            ..RootfsLimits::default()
        },
        RootfsLimits {
            max_entries: 1,
            ..RootfsLimits::default()
        },
    ] {
        let (fixture, imported) = import_layers(std::slice::from_ref(&layer));
        assert_kind(
            run(&fixture, &imported, limits),
            NormalizeErrorKind::LimitExceeded,
        );
    }
}

#[test]
fn path_metadata_file_and_aggregate_content_budgets_are_enforced_while_streaming() {
    let layer = tar(&[
        TarEntry::file(b"long-path", b"12"),
        TarEntry::file(b"other", b"34"),
    ]);
    for limits in [
        RootfsLimits {
            max_path_bytes: 4,
            ..RootfsLimits::default()
        },
        RootfsLimits {
            max_metadata_bytes: 5,
            ..RootfsLimits::default()
        },
        RootfsLimits {
            max_file_bytes: 1,
            ..RootfsLimits::default()
        },
        RootfsLimits {
            max_content_bytes: 3,
            ..RootfsLimits::default()
        },
    ] {
        let (fixture, imported) = import_layers(std::slice::from_ref(&layer));
        assert_kind(
            run(&fixture, &imported, limits),
            NormalizeErrorKind::LimitExceeded,
        );
    }
}

#[test]
fn implicit_parent_entries_and_paths_are_bounded_during_tree_mutation() {
    let layer = tar(&[TarEntry::file(b"a/b/c/d/value", b"body")]);
    let cases = [
        RootfsLimits {
            max_entries: 3,
            ..RootfsLimits::default()
        },
        RootfsLimits {
            max_metadata_bytes: 13,
            ..RootfsLimits::default()
        },
    ];
    for limits in cases {
        let (fixture, imported) = import_layers(std::slice::from_ref(&layer));
        assert_kind(
            run(&fixture, &imported, limits),
            NormalizeErrorKind::LimitExceeded,
        );
    }
}

#[test]
fn canonical_tree_manifest_has_an_independent_checked_output_bound() {
    let entries: Vec<_> = (0_u8..32)
        .map(|index| (format!("file-{index:02}"), vec![index]))
        .collect();
    let specs: Vec<_> = entries
        .iter()
        .map(|(path, body)| TarEntry::file(path.as_bytes(), body))
        .collect();
    let layer = tar(&specs);
    let (fixture, imported) = import_layers(&[layer]);
    let complete = run(&fixture, &imported, RootfsLimits::default()).unwrap();
    assert!(complete.tree_manifest_size() > imported.import_manifest_size());
    let limits = RootfsLimits {
        max_manifest_bytes: complete.tree_manifest_size() - 1,
        ..RootfsLimits::default()
    };

    assert_kind(
        run(&fixture, &imported, limits),
        NormalizeErrorKind::LimitExceeded,
    );
}

#[test]
fn raw_header_budget_is_shared_across_selected_layers() {
    let layers = [
        local_pax_layer(
            &TarEntry::file(b"placeholder", b"one"),
            &[("path", b"first")],
        ),
        local_pax_layer(
            &TarEntry::file(b"placeholder", b"two"),
            &[("path", b"second")],
        ),
    ];
    let (fixture, imported) = import_layers(&layers);
    let limits = RootfsLimits {
        max_entries: 3,
        ..RootfsLimits::default()
    };

    assert_kind(
        run(&fixture, &imported, limits),
        NormalizeErrorKind::LimitExceeded,
    );
}

#[test]
fn every_zero_request_bound_is_invalid() {
    let layer = tar(&[TarEntry::file(b"value", b"body")]);
    let setters: [fn(&mut RootfsLimits); 8] = [
        |limits| limits.max_blob_bytes = 0,
        |limits| limits.max_expanded_bytes = 0,
        |limits| limits.max_entries = 0,
        |limits| limits.max_path_bytes = 0,
        |limits| limits.max_metadata_bytes = 0,
        |limits| limits.max_file_bytes = 0,
        |limits| limits.max_content_bytes = 0,
        |limits| limits.max_manifest_bytes = 0,
    ];
    for set_zero in setters {
        let (fixture, imported) = import_layers(std::slice::from_ref(&layer));
        let mut limits = RootfsLimits::default();
        set_zero(&mut limits);
        assert_kind(
            run(&fixture, &imported, limits),
            NormalizeErrorKind::InvalidInput,
        );
    }
}

fn import_layers(layers: &[Vec<u8>]) -> (Fixture, ImportedOci) {
    let fixture = Fixture::new();
    let image = add_layers(&fixture, layers);
    fixture.write_direct_index(&image, true);
    let imported = import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Platform(&OciPlatform::linux_arm64()),
        ImportLimits::default(),
    ))
    .unwrap();
    (fixture, imported)
}

fn run(
    fixture: &Fixture,
    imported: &ImportedOci,
    limits: RootfsLimits,
) -> Result<soma_generation::NormalizedRootfs, soma_generation::NormalizeError> {
    normalize_oci_rootfs(NormalizeOciRootfs::new(imported, &fixture.store, limits))
}

fn assert_kind<T: std::fmt::Debug>(
    result: Result<T, soma_generation::NormalizeError>,
    expected: NormalizeErrorKind,
) {
    assert_eq!(result.unwrap_err().kind(), expected);
}
