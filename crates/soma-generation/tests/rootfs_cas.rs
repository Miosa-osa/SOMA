mod support;

use std::{fs, io::Write as _};

use flate2::{Compression, write::GzEncoder};
use soma::OciPlatform;
use soma_generation::{
    ImportLimits, ImportOciLayout, ImportedOci, NormalizeErrorKind, NormalizeOciRootfs,
    OciSelection, RootfsLimits, import_oci_layout, normalize_oci_rootfs,
};
use support::{
    Fixture, GZIP, PLAIN,
    rootfs::{TarEntry, add_layers, tar},
};
use tar::EntryType;

#[test]
fn import_manifest_and_layer_objects_are_reverified_before_use() {
    let layer = tar(&[TarEntry::file(b"value", b"body")]);
    let (fixture, imported) = import_layers(&[layer]);
    tamper(&blob(&fixture, imported.import_manifest_digest()));
    assert_kind(run(&fixture, &imported), NormalizeErrorKind::StoreConflict);

    let layer = tar(&[TarEntry::file(b"value", b"body")]);
    let (fixture, imported) = import_layers(&[layer]);
    let import_bytes = fs::read(blob(&fixture, imported.import_manifest_digest())).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&import_bytes).unwrap();
    let layer_digest =
        soma::OciDigest::parse(manifest["layers"][0]["blob"]["digest"].as_str().unwrap()).unwrap();
    tamper(&blob(&fixture, &layer_digest));
    assert_kind(run(&fixture, &imported), NormalizeErrorKind::StoreConflict);
}

#[test]
fn gzip_layer_is_revalidated_and_normalized_through_the_verified_handle() {
    let fixture = Fixture::new();
    let expanded = tar(&[TarEntry::file(b"gzip", b"expanded")]);
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&expanded).unwrap();
    let compressed = encoder.finish().unwrap();
    let image = fixture.add_image(&compressed, &expanded, GZIP);
    fixture.write_direct_index(&image, true);
    let imported = import_fixture(&fixture);

    let normalized = run(&fixture, &imported).unwrap();
    // Two entries from the layer, plus the five mount points every SOMA root carries.
    assert_eq!(
        (normalized.entry_count(), normalized.logical_file_bytes()),
        (7, 8)
    );
}

#[test]
fn plain_and_gzip_transport_produce_the_same_normalized_tree_identity() {
    let expanded = tar(&[TarEntry::file(b"transport", b"same tree")]);
    let plain_fixture = Fixture::new();
    let plain_image = plain_fixture.add_image(&expanded, &expanded, PLAIN);
    plain_fixture.write_direct_index(&plain_image, true);
    let plain_import = import_fixture(&plain_fixture);
    let plain = run(&plain_fixture, &plain_import).unwrap();

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&expanded).unwrap();
    let compressed = encoder.finish().unwrap();
    let gzip_fixture = Fixture::new();
    let gzip_image = gzip_fixture.add_image(&compressed, &expanded, GZIP);
    gzip_fixture.write_direct_index(&gzip_image, true);
    let gzip_import = import_fixture(&gzip_fixture);
    let gzip = run(&gzip_fixture, &gzip_import).unwrap();

    assert_ne!(
        plain.source_import_manifest_digest(),
        gzip.source_import_manifest_digest()
    );
    assert_eq!(plain.tree_manifest_digest(), gzip.tree_manifest_digest());
}

#[test]
fn failed_late_layer_can_leave_content_but_never_publishes_completion() {
    let layer = tar(&[
        TarEntry::file(b"content", b"published first"),
        TarEntry::special(b"device", EntryType::Char),
    ]);
    let (fixture, imported) = import_layers(&[layer]);
    let before = blob_count(&fixture);

    assert_kind(run(&fixture, &imported), NormalizeErrorKind::Unsupported);

    assert_eq!(blob_count(&fixture), before + 1);
    assert!(
        fs::read_dir(fixture.store.join("v1/blobs/sha256"))
            .unwrap()
            .all(|entry| !fs::read(entry.unwrap().path())
                .unwrap()
                .starts_with(b"SOMARFS\0"))
    );
    assert_eq!(
        fs::read_dir(fixture.store.join("v1/tmp")).unwrap().count(),
        0
    );
}

#[test]
fn concurrent_normalizers_publish_one_idempotent_completion() {
    let layer = tar(&[TarEntry::file(b"value", b"content")]);
    let (fixture, imported) = import_layers(&[layer]);

    let digests = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| run(&fixture, &imported)))
            .collect();
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .unwrap()
                    .unwrap()
                    .tree_manifest_digest()
                    .clone()
            })
            .collect::<Vec<_>>()
    });

    assert!(digests.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(blob_count(&fixture), 7);
    assert_eq!(
        fs::read_dir(fixture.store.join("v1/tmp")).unwrap().count(),
        0
    );
}

#[test]
fn request_result_and_errors_redact_host_and_guest_paths() {
    let layer = tar(&[
        TarEntry::symlink(b"tenant-secret", b"outside-secret"),
        TarEntry::file(b"tenant-secret/child", b"body"),
    ]);
    let (fixture, imported) = import_layers(&[layer]);
    let request = NormalizeOciRootfs::new(&imported, &fixture.store, RootfsLimits::default());
    assert!(!format!("{request:?}").contains(fixture.store.to_str().unwrap()));
    let error = normalize_oci_rootfs(request).unwrap_err();
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("tenant-secret"));
    assert!(!rendered.contains("outside-secret"));
    assert!(!rendered.contains(fixture.store.to_str().unwrap()));

    let layer = tar(&[TarEntry::file(b"another-secret", b"body")]);
    let (fixture, imported) = import_layers(&[layer]);
    let result = run(&fixture, &imported).unwrap();
    assert!(!format!("{result:?}").contains("another-secret"));
}

fn import_layers(layers: &[Vec<u8>]) -> (Fixture, ImportedOci) {
    let fixture = Fixture::new();
    let image = add_layers(&fixture, layers);
    fixture.write_direct_index(&image, true);
    let imported = import_fixture(&fixture);
    (fixture, imported)
}

fn import_fixture(fixture: &Fixture) -> ImportedOci {
    import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Platform(&OciPlatform::linux_arm64()),
        ImportLimits::default(),
    ))
    .unwrap()
}

fn run(
    fixture: &Fixture,
    imported: &ImportedOci,
) -> Result<soma_generation::NormalizedRootfs, soma_generation::NormalizeError> {
    normalize_oci_rootfs(NormalizeOciRootfs::new(
        imported,
        &fixture.store,
        RootfsLimits::default(),
    ))
}

fn blob(fixture: &Fixture, digest: &soma::OciDigest) -> std::path::PathBuf {
    fixture
        .store
        .join("v1/blobs/sha256")
        .join(&digest.as_str()[7..])
}

fn blob_count(fixture: &Fixture) -> usize {
    fs::read_dir(fixture.store.join("v1/blobs/sha256"))
        .unwrap()
        .count()
}

fn assert_kind<T: std::fmt::Debug>(
    result: Result<T, soma_generation::NormalizeError>,
    expected: NormalizeErrorKind,
) {
    assert_eq!(result.unwrap_err().kind(), expected);
}

fn tamper(path: &std::path::Path) {
    make_writable(path);
    let mut bytes = fs::read(path).unwrap();
    bytes[0] ^= 1;
    fs::write(path, bytes).unwrap();
}

#[cfg(unix)]
fn make_writable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions).unwrap();
}

#[cfg(windows)]
#[allow(
    clippy::permissions_set_readonly_false,
    reason = "test mutates its private fixture"
)]
fn make_writable(path: &std::path::Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions).unwrap();
}
