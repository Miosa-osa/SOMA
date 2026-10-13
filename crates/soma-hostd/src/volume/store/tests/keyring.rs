//! The data key's place in the store's lifecycle: sealed at create, destroyed with the volume.

use super::*;
use crate::volume::keyring::HostKeyring;

/// A store whose host key lives in this test's directory rather than on an encrypted mount.
fn store_with_keys(directory: &tempfile::TempDir) -> DirectoryVolumes {
    let records = directory.path().join("records");
    DirectoryVolumes::open_uncertified(
        &directory.path().join("volumes"),
        &records,
        Box::new(CleanRepair),
        true,
    )
    .expect("opens")
    .with_keys(Box::new(HostKeyring::at(
        directory.path().join("volume-master.key"),
        &records,
    )))
}

#[test]
fn a_created_volume_carries_a_data_key_that_its_destruction_removes() {
    let directory = tempfile::tempdir().expect("temp dir");
    let volumes = store_with_keys(&directory);
    let sealed = directory
        .path()
        .join("records")
        .join("keys")
        .join(volume(1).hex())
        .join("dek");

    volumes.attach(&attach(1, 1, 1, 10)).expect("attach");
    let sidecar = volumes.inspect(volume(1)).expect("exists");
    assert_ne!(
        sidecar.dek_id, [0; 16],
        "a created volume names the key it was sealed under"
    );
    assert!(
        sealed.exists(),
        "the sealed key lives under the durable record root, not in the volume directory"
    );
    assert!(
        !directory
            .path()
            .join("volumes")
            .join(volume(1).hex())
            .join("dek")
            .exists(),
        "no key material is written into the volume's own directory"
    );

    volumes.destroy(volume(1)).expect("destroy");
    assert!(
        !sealed.exists(),
        "destroying a volume destroys the key it was written under"
    );
    assert_eq!(
        volumes.inspect(volume(1)).err(),
        Some(VolumeError::Missing("no sidecar names this volume"))
    );
}

#[test]
fn a_host_that_cannot_hold_a_key_refuses_the_volume_rather_than_creating_it() {
    let directory = tempfile::tempdir().expect("temp dir");
    let records = directory.path().join("records");
    // The master key's parent is a regular file, so no key can be created anywhere.
    std::fs::write(directory.path().join("a-file"), b"x").expect("write");
    let volumes = DirectoryVolumes::open_uncertified(
        &directory.path().join("volumes"),
        &records,
        Box::new(CleanRepair),
        true,
    )
    .expect("opens")
    .with_keys(Box::new(HostKeyring::at(
        directory.path().join("a-file").join("volume-master.key"),
        &records,
    )));

    assert_eq!(
        volumes.attach(&attach(1, 1, 1, 10)).err(),
        Some(VolumeError::Encryption(
            "the volume's data key could not be sealed"
        ))
    );
    assert_eq!(
        volumes.inspect(volume(1)).err(),
        Some(VolumeError::Missing("no sidecar names this volume")),
        "a volume whose key could not be sealed is not created"
    );
}
