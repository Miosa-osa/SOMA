//! The keyring's own tests: the host key, the seal, and the destruction.

use std::fs;
use std::path::Path;

use std::os::unix::fs::PermissionsExt as _;

use super::*;

fn volume() -> VolumeId {
    VolumeId::new([4; 16]).expect("nonzero")
}

fn keyring(directory: &Path) -> HostKeyring {
    HostKeyring::at(directory.join("volume-master.key"), directory)
}

#[test]
fn the_master_key_is_created_root_only_and_reused_rather_than_replaced() {
    let directory = tempfile::tempdir().expect("temp dir");
    let keys = keyring(directory.path());
    let first = keys.seal(volume()).expect("seals");
    let master = directory.path().join("volume-master.key");
    let metadata = fs::metadata(&master).expect("the host key exists");
    assert_eq!(metadata.len(), KEY_LEN as u64);
    assert_eq!(
        metadata.permissions().mode() & 0o777,
        0o600,
        "the host key is readable by its owner alone"
    );

    // A second seal uses the same master key and a fresh data key, so two volumes never
    // share one key even though one host key protects both.
    let second = keys.seal(volume()).expect("seals");
    assert_ne!(first.sealed, second.sealed);
    assert_ne!(first.dek_id, second.dek_id);
    assert_eq!(keys.unseal(&first).expect("unseals").len(), KEY_LEN);
}

#[test]
fn a_sealed_key_opens_under_its_master_key_and_a_tampered_one_does_not() {
    let directory = tempfile::tempdir().expect("temp dir");
    let keys = keyring(directory.path());
    let sealed = keys.seal(volume()).expect("seals");
    assert!(keys.unseal(&sealed).is_ok());

    let mut tampered = sealed;
    tampered.sealed[0] ^= 0x01;
    assert_eq!(keys.unseal(&tampered), Err(KeyError::Seal));

    // The identity is a digest of the frame, so a changed frame read back under the same name is
    // a different key rather than one that passes as the key it replaced.
    keys.record(volume(), &tampered).expect("records");
    assert_ne!(keys.read(volume()).expect("reads").dek_id, sealed.dek_id);
}

#[test]
fn a_sealed_key_round_trips_through_the_record_root_and_a_destroy_removes_it() {
    let directory = tempfile::tempdir().expect("temp dir");
    let keys = keyring(directory.path());
    let sealed = keys.seal(volume()).expect("seals");
    keys.record(volume(), &sealed).expect("records");
    let stored = keys.read(volume()).expect("reads");
    assert_eq!(stored, sealed);
    assert!(keys.unseal(&stored).is_ok());

    keys.destroy(volume()).expect("destroys");
    assert_eq!(keys.read(volume()), Err(KeyError::Missing));
    assert!(!keys.sealed_path(volume()).exists());
    // A destroy that is replayed after a lost answer is the state it wanted.
    keys.destroy(volume()).expect("a replay is not an error");
}

#[test]
fn a_master_key_of_the_wrong_length_is_refused_rather_than_used_short() {
    let directory = tempfile::tempdir().expect("temp dir");
    fs::write(directory.path().join("volume-master.key"), b"short").expect("write");
    let keys = keyring(directory.path());
    assert_eq!(
        keys.seal(volume()),
        Err(KeyError::KeyFile("the host key could not be read"))
    );
}
