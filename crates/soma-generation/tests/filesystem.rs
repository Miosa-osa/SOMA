mod support;

use std::fs;

use soma::OciPlatform;
use soma_generation::{
    ImportErrorKind, ImportLimits, ImportOciLayout, OciSelection, import_oci_layout,
};
use support::Fixture;

#[test]
fn corrupted_existing_cas_object_is_never_overwritten() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    fixture.write_direct_index(&image, true);
    let blobs = fixture.store.join("v1/blobs/sha256");
    fs::create_dir_all(&blobs).unwrap();
    let target = blobs.join(&image.layer_digest[7..]);
    fs::write(&target, b"wrong").unwrap();

    let error = run(&fixture).unwrap_err();

    assert_eq!(error.kind(), ImportErrorKind::StoreConflict);
    assert_eq!(fs::read(target).unwrap(), b"wrong");
}

#[cfg(unix)]
#[test]
fn source_blob_symlink_is_not_followed() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    let manifest_path = fixture.blob_path(&image.manifest_digest);
    let outside = fixture.keepalive().join("outside");
    fs::rename(&manifest_path, &outside).unwrap();
    symlink(&outside, &manifest_path).unwrap();
    fixture.write_direct_index(&image, true);

    assert!(run(&fixture).is_err());
}

#[cfg(unix)]
#[test]
fn store_directory_symlink_is_not_followed() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    fixture.write_direct_index(&image, true);
    fs::create_dir(fixture.store.join("v1")).unwrap();
    let outside = fixture.keepalive().join("outside-store");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, fixture.store.join("v1/blobs")).unwrap();

    let error = run(&fixture).unwrap_err();

    assert_eq!(error.kind(), ImportErrorKind::StoreConflict);
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[test]
fn layout_root_symlink_is_rejected() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    fixture.write_direct_index(&image, true);
    let linked_layout = fixture.keepalive().join("linked-layout");
    if !symlink_directory(&fixture.layout, &linked_layout) {
        return;
    }

    let error = run_paths(&linked_layout, &fixture.store).unwrap_err();

    assert_eq!(error.kind(), ImportErrorKind::InvalidInput);
}

#[test]
fn store_root_symlink_is_rejected_without_writing_its_target() {
    let fixture = Fixture::new();
    let image = fixture.add_plain_image(b"layer");
    fixture.write_direct_index(&image, true);
    let store_target = fixture.keepalive().join("store-target");
    fs::create_dir(&store_target).unwrap();
    let linked_store = fixture.keepalive().join("linked-store");
    if !symlink_directory(&store_target, &linked_store) {
        return;
    }

    let error = run_paths(&fixture.layout, &linked_store).unwrap_err();

    assert_eq!(error.kind(), ImportErrorKind::StoreConflict);
    assert_eq!(fs::read_dir(store_target).unwrap().count(), 0);
}

fn run(fixture: &Fixture) -> Result<soma_generation::ImportedOci, soma_generation::ImportError> {
    run_paths(&fixture.layout, &fixture.store)
}

fn run_paths(
    layout: &std::path::Path,
    store: &std::path::Path,
) -> Result<soma_generation::ImportedOci, soma_generation::ImportError> {
    import_oci_layout(ImportOciLayout::new(
        layout,
        store,
        OciSelection::Platform(&OciPlatform::linux_arm64()),
        ImportLimits::default(),
    ))
}

#[cfg(unix)]
fn symlink_directory(source: &std::path::Path, target: &std::path::Path) -> bool {
    std::os::unix::fs::symlink(source, target).unwrap();
    true
}

#[cfg(windows)]
fn symlink_directory(source: &std::path::Path, target: &std::path::Path) -> bool {
    match std::os::windows::fs::symlink_dir(source, target) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => false,
        Err(error) => panic!("could not create directory symlink: {error}"),
    }
}

#[cfg(not(any(unix, windows)))]
fn symlink_directory(_: &std::path::Path, _: &std::path::Path) -> bool {
    false
}
