mod support;

use std::fs;

use serde_json::json;
use soma::OciPlatform;
use soma_generation::{ImportLimits, ImportOciLayout, OciSelection, import_oci_layout};
use support::{CONFIG, Fixture, Image, MANIFEST, PLAIN, descriptor, digest, tar_layer};

#[test]
fn repeated_import_is_idempotent_and_leaves_no_temporary_files() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    fixture.write_direct_index(&image, true);

    let first = run(&fixture).unwrap();
    let second = run(&fixture).unwrap();

    assert_eq!(
        first.import_manifest_digest(),
        second.import_manifest_digest()
    );
    assert_eq!(first.stored_blob_count(), second.stored_blob_count());
    assert_eq!(
        fs::read_dir(fixture.store.join("v1/tmp")).unwrap().count(),
        0
    );
    let artifact = fixture
        .store
        .join("v1/blobs/sha256")
        .join(&first.import_manifest_digest().as_str()[7..]);
    assert!(fs::metadata(artifact).unwrap().permissions().readonly());
}

#[test]
fn repeated_import_restores_readonly_existing_object() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    fixture.write_direct_index(&image, true);
    let first = run(&fixture).unwrap();
    let layer = fixture
        .store
        .join("v1/blobs/sha256")
        .join(&image.layer_digest[7..]);
    make_writable(&layer);
    assert!(!fs::metadata(&layer).unwrap().permissions().readonly());

    let second = run(&fixture).unwrap();

    assert_eq!(
        first.import_manifest_digest(),
        second.import_manifest_digest()
    );
    assert!(fs::metadata(layer).unwrap().permissions().readonly());
}

#[test]
fn failed_expansion_never_publishes_a_completion_manifest() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"12345");
    fixture.write_direct_index(&image, true);
    let limits = ImportLimits {
        max_expanded_bytes: 4,
        ..ImportLimits::default()
    };

    let result = import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Platform(&OciPlatform::linux_arm64()),
        limits,
    ));

    assert!(result.is_err());
    assert!(
        !fixture
            .store
            .join("v1/blobs/sha256")
            .join(&image.layer_digest[7..])
            .exists()
    );
    assert_eq!(
        fs::read_dir(fixture.store.join("v1/blobs/sha256"))
            .unwrap()
            .count(),
        3
    );
    assert_eq!(
        fs::read_dir(fixture.store.join("v1/tmp")).unwrap().count(),
        0
    );
}

#[test]
fn late_diff_id_failure_publishes_no_selected_layers() {
    let fixture = Fixture::new();
    let first = tar_layer(b"first-layer");
    let second = tar_layer(b"second-layer");
    let first_digest = fixture.put_blob(&first);
    let second_digest = fixture.put_blob(&second);
    let config = serde_json::to_vec(&json!({
        "architecture": "arm64",
        "os": "linux",
        "rootfs": {
            "type": "layers",
            "diff_ids": [digest(&first), digest(b"wrong-second-layer")],
        },
    }))
    .unwrap();
    let config_digest = fixture.put_blob(&config);
    let manifest = serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": MANIFEST,
        "config": descriptor(CONFIG, &config_digest, config.len()),
        "layers": [
            descriptor(PLAIN, &first_digest, first.len()),
            descriptor(PLAIN, &second_digest, second.len()),
        ],
    }))
    .unwrap();
    let manifest_digest = fixture.put_blob(&manifest);
    fixture.write_direct_index(
        &Image {
            manifest_digest,
            manifest_size: manifest.len(),
            config_digest,
            layer_digest: first_digest.clone(),
        },
        true,
    );

    assert!(run(&fixture).is_err());
    let blobs = fixture.store.join("v1/blobs/sha256");
    assert!(!blobs.join(&first_digest[7..]).exists());
    assert!(!blobs.join(&second_digest[7..]).exists());
    assert_eq!(fs::read_dir(blobs).unwrap().count(), 3);
    assert_eq!(
        fs::read_dir(fixture.store.join("v1/tmp")).unwrap().count(),
        0
    );
}

#[test]
fn concurrent_importers_publish_one_identical_completion_object() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    fixture.write_direct_index(&image, true);

    let digests = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4).map(|_| scope.spawn(|| run(&fixture))).collect();
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .unwrap()
                    .unwrap()
                    .import_manifest_digest()
                    .clone()
            })
            .collect::<Vec<_>>()
    });

    assert!(digests.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(
        fs::read_dir(fixture.store.join("v1/blobs/sha256"))
            .unwrap()
            .count(),
        5
    );
    assert_eq!(
        fs::read_dir(fixture.store.join("v1/tmp")).unwrap().count(),
        0
    );
}

#[cfg(windows)]
#[test]
fn native_windows_publication_does_not_require_directory_fsync() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    fixture.write_direct_index(&image, true);

    let imported = run(&fixture).unwrap();

    assert_eq!(imported.stored_blob_count(), 5);
    let artifact = fixture
        .store
        .join("v1/blobs/sha256")
        .join(&imported.import_manifest_digest().as_str()[7..]);
    assert!(fs::metadata(artifact).unwrap().permissions().readonly());
    assert_eq!(
        fs::read_dir(fixture.store.join("v1/tmp")).unwrap().count(),
        0
    );
}

fn run(fixture: &Fixture) -> Result<soma_generation::ImportedOci, soma_generation::ImportError> {
    import_oci_layout(ImportOciLayout::new(
        &fixture.layout,
        &fixture.store,
        OciSelection::Platform(&OciPlatform::linux_arm64()),
        ImportLimits::default(),
    ))
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
    reason = "Windows exposes a read-only attribute rather than Unix mode bits"
)]
fn make_writable(path: &std::path::Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions).unwrap();
}
