//! What a start binds: the volume an Instance owns, its file, and this start's repair.

use super::*;

/// A double whose repair always reports damage the automatic policy will not touch.
struct RefusingRepair;

impl VolumeRepair for RefusingRepair {
    fn repair(&self, volume: VolumeId, _file: &Path) -> RepairReceipt {
        RepairReceipt::new(volume, RepairOutcome::Refused, 4, 0)
    }
}

#[test]
fn a_binding_opens_the_volume_attached_to_its_instance_and_reports_this_starts_repair() {
    let directory = tempfile::tempdir().expect("temp dir");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let volumes = with_repairs(
        &directory,
        Box::new(Recording {
            calls: Arc::clone(&calls),
        }),
    );
    // Nothing is attached yet, so there is nothing a machine may be given.
    assert_eq!(
        volumes.binding(instance(1)).err(),
        Some(VolumeError::Missing(
            "no volume is attached to this Instance"
        ))
    );

    volumes.attach(&attach(1, 1, 1, 10)).expect("attach");
    calls.lock().expect("lock").clear();
    let binding = volumes.binding(instance(1)).expect("binds");
    assert_eq!(binding.volume, volume(1));
    assert_eq!(binding.size_class_gib, 10);
    assert_eq!(binding.path, volumes.file(volume(1)));
    assert_eq!(binding.repair.outcome, RepairOutcome::Clean);
    assert_eq!(
        *calls.lock().expect("lock"),
        vec!["repair"],
        "a start repairs the detached file before any machine holds it"
    );

    // A detached volume is not handed to a machine: the stop is what makes it a file again.
    volumes.detach(&detach(1, 1, 2)).expect("detach");
    assert!(matches!(
        volumes.binding(instance(1)),
        Err(VolumeError::Missing(_))
    ));
}

#[test]
fn a_binding_refuses_a_volume_this_starts_repair_cannot_make_usable() {
    let directory = tempfile::tempdir().expect("temp dir");
    let volumes = with_repairs(
        &directory,
        Box::new(Recording {
            calls: Arc::new(Mutex::new(Vec::new())),
        }),
    );
    volumes.attach(&attach(1, 1, 1, 10)).expect("attach");

    // The same root with a repair that refuses: a start must not hand a machine a file the host
    // could not make consistent.
    let refusing = with_repairs(&directory, Box::new(RefusingRepair));
    assert_eq!(
        refusing.binding(instance(1)).err(),
        Some(VolumeError::RepairRefused(
            "the volume needs a repair beyond the policy"
        ))
    );
    assert!(volumes.binding(instance(1)).is_ok());
}

#[test]
fn a_binding_names_the_volume_of_its_own_instance_and_no_other() {
    let directory = tempfile::tempdir().expect("temp dir");
    let volumes = with_repairs(
        &directory,
        Box::new(Recording {
            calls: Arc::new(Mutex::new(Vec::new())),
        }),
    );
    volumes.attach(&attach(1, 7, 1, 10)).expect("attach");
    assert!(volumes.binding(instance(7)).is_ok());
    assert_eq!(
        volumes.binding(instance(8)).err(),
        Some(VolumeError::Missing(
            "no volume is attached to this Instance"
        ))
    );
}

#[test]
fn a_binding_refuses_a_root_that_is_not_on_the_encrypted_data_mount() {
    // The same rule as an attach: neither one hands a tenant's bytes to a machine from a root
    // that is not certified.
    let directory = tempfile::tempdir().expect("temp dir");
    let volumes = DirectoryVolumes::open_uncertified(
        &directory.path().join("volumes"),
        &directory.path().join("records"),
        Box::new(Recording {
            calls: Arc::new(Mutex::new(Vec::new())),
        }),
        false,
    )
    .expect("opens");
    assert_eq!(
        volumes.binding(instance(1)).err(),
        Some(VolumeError::Encryption(
            "the volume root is not on this host's encrypted data mount"
        ))
    );
}
