//! The encrypted-root rule: a host that is not on its data mount refuses every attach.

use super::*;

#[test]
fn a_host_without_an_encrypted_data_mount_refuses_to_attach() {
    let directory = tempfile::tempdir().expect("temp dir");
    // The unencrypted constructor is the one a host that holds no tenant data opens, and what it
    // certifies is false: the attach is refused before anything is created.
    let volumes = DirectoryVolumes::open_uncertified(
        &directory.path().join("volumes"),
        &directory.path().join("records"),
        Box::new(CleanRepair),
        false,
    )
    .expect("opens");
    assert_eq!(
        volumes.attach(&attach(1, 1, 1, 10)).err(),
        Some(VolumeError::Encryption(
            "the volume root is not on this host's encrypted data mount"
        ))
    );
}

#[test]
fn the_certifying_constructor_refuses_exactly_the_roots_that_are_not_on_the_data_mount() {
    use crate::volume::placement::{DEFAULT_ENCRYPTED_MOUNT, root_is_encrypted};

    let directory = tempfile::tempdir().expect("temp dir");
    let volumes = DirectoryVolumes::open(
        &directory.path().join("volumes"),
        &directory.path().join("records"),
        Box::new(CleanRepair),
    )
    .expect("opens");
    let certified = root_is_encrypted(directory.path(), DEFAULT_ENCRYPTED_MOUNT)
        .expect("the mount table is readable on a host that runs this store");
    // The rule is about the mount the test's own directory sits on, so the expectation is read
    // from the same place the store reads it rather than hard-coded to this host.
    let outcome = volumes.attach(&attach(1, 1, 1, 10));
    if certified {
        assert!(outcome.is_ok(), "a certified root attaches");
    } else {
        assert!(
            matches!(outcome, Err(VolumeError::Encryption(_))),
            "a root that is not on the data mount is refused"
        );
    }
}
