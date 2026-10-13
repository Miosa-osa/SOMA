use super::*;

use std::sync::{Arc, Mutex};

use crate::volume::repair::CreateError;
use crate::{GenerationId, InstanceId};

fn instance(byte: u8) -> InstanceId {
    InstanceId::new([byte; 16]).expect("nonzero")
}

fn volume(byte: u8) -> VolumeId {
    VolumeId::new([byte; 16]).expect("nonzero")
}

fn operation(byte: u8) -> OperationId {
    OperationId::new([byte; 16]).expect("nonzero")
}

fn fingerprint(byte: u8) -> RequestFingerprint {
    RequestFingerprint::new([byte; 32]).expect("nonzero")
}

fn attach(volume_byte: u8, instance_byte: u8, operation_byte: u8, class: u32) -> AttachRequest {
    AttachRequest {
        operation: operation(operation_byte),
        instance: instance(instance_byte),
        volume: volume(volume_byte),
        fingerprint: fingerprint(operation_byte),
        size_class_gib: class,
        generation: GenerationId::new([1; 32]).expect("nonzero"),
        tenant: "tenant-alpha".to_owned(),
    }
}

fn detach(volume_byte: u8, instance_byte: u8, operation_byte: u8) -> DetachRequest {
    DetachRequest {
        operation: operation(operation_byte),
        instance: instance(instance_byte),
        volume: volume(volume_byte),
        fingerprint: fingerprint(operation_byte),
    }
}

fn store() -> (tempfile::TempDir, DirectoryVolumes) {
    let directory = tempfile::tempdir().expect("temp dir");
    let volumes = DirectoryVolumes::open_uncertified(
        &directory.path().join("volumes"),
        &directory.path().join("records"),
        Box::new(CleanRepair),
        true,
    )
    .expect("opens");
    (directory, volumes)
}

fn record_count(root: &Path, volume_id: VolumeId) -> usize {
    match std::fs::read_dir(root.join("records").join(volume_id.hex())) {
        Ok(entries) => entries.count(),
        Err(_) => 0,
    }
}

#[test]
fn a_fresh_attach_creates_the_volume_and_reports_a_usable_repair() {
    let (work, volumes) = store();
    let outcome = volumes.attach(&attach(1, 9, 1, 10)).expect("attaches");
    assert_eq!(outcome.state, VolumeState::Attached);
    assert_eq!(outcome.device, VOLUME_DEVICE);
    assert!(outcome.repair.expect("a receipt").is_usable());
    assert!(volumes.directory(volume(1)).join(SIDECAR_FILE).exists());
    // Create begin and end, then attach begin and end.
    assert_eq!(record_count(work.path(), volume(1)), 4);
}

#[test]
fn a_replayed_attach_under_the_same_fingerprint_is_idempotent() {
    let (_work, volumes) = store();
    let first = volumes.attach(&attach(1, 9, 1, 10)).expect("attaches");
    let again = volumes.attach(&attach(1, 9, 1, 10)).expect("replays");
    assert_eq!(first.volume, again.volume);
    assert_eq!(
        volumes.inspect(volume(1)).expect("exists").state,
        VolumeState::Attached
    );
}

#[test]
fn a_replay_that_changes_the_intent_is_refused() {
    let (_work, volumes) = store();
    volumes.attach(&attach(1, 9, 1, 10)).expect("attaches");
    let mut changed = attach(1, 9, 1, 10);
    changed.fingerprint = fingerprint(2);
    assert_eq!(volumes.attach(&changed), Err(VolumeError::ChangedIntent));
}

#[test]
fn a_volume_owned_by_another_instance_is_busy() {
    let (_work, volumes) = store();
    volumes.attach(&attach(1, 9, 1, 10)).expect("attaches");
    assert_eq!(
        volumes.attach(&attach(1, 8, 2, 10)),
        Err(VolumeError::Busy("the volume belongs to another Instance"))
    );
}

#[test]
fn a_class_below_the_volume_own_is_refused() {
    let (_work, volumes) = store();
    volumes.attach(&attach(1, 9, 1, 20)).expect("attaches");
    assert_eq!(
        volumes.attach(&attach(1, 9, 2, 10)),
        Err(VolumeError::Geometry(
            "the requested class is below the volume's own"
        ))
    );
}

#[test]
fn a_detach_is_recorded_and_idempotent() {
    let (work, volumes) = store();
    volumes.attach(&attach(1, 9, 1, 10)).expect("attaches");
    let detached = volumes.detach(&detach(1, 9, 2)).expect("detaches");
    assert_eq!(detached.state, VolumeState::Detached);
    assert_eq!(
        volumes.inspect(volume(1)).expect("exists").state,
        VolumeState::Detached
    );
    assert_eq!(record_count(work.path(), volume(1)), 6);
}

#[test]
fn a_detach_of_an_unknown_volume_is_missing() {
    let (_work, volumes) = store();
    assert_eq!(
        volumes.detach(&detach(1, 9, 2)),
        Err(VolumeError::Missing("no sidecar names this volume"))
    );
}

#[test]
fn a_destroy_removes_the_volume_and_a_later_inspect_is_missing() {
    let (_work, volumes) = store();
    volumes.attach(&attach(1, 9, 1, 10)).expect("attaches");
    volumes.destroy(volume(1)).expect("destroys");
    assert!(volumes.inspect(volume(1)).is_err());
    assert!(!volumes.directory(volume(1)).exists());
}

#[test]
fn a_reopened_store_rebuilds_the_volume_from_its_sidecar() {
    let directory = tempfile::tempdir().expect("temp dir");
    let root = directory.path().join("volumes");
    let records = directory.path().join("records");
    {
        let volumes =
            DirectoryVolumes::open_uncertified(&root, &records, Box::new(CleanRepair), true)
                .expect("opens");
        volumes.attach(&attach(1, 9, 1, 10)).expect("attaches");
    }
    let reopened = DirectoryVolumes::open_uncertified(&root, &records, Box::new(CleanRepair), true)
        .expect("opens");
    let sidecar = reopened.inspect(volume(1)).expect("exists");
    assert_eq!(sidecar.state, VolumeState::Attached);
    assert_eq!(sidecar.tenant, "tenant-alpha");
    assert_eq!(sidecar.size_class_gib, 10);
}

/// A double that records the two filesystem operations in the order the store runs them.
struct Recording {
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl VolumeRepair for Recording {
    fn create(&self, _volume: VolumeId, _file: &Path, size_bytes: u64) -> Result<(), CreateError> {
        assert!(size_bytes > 0, "a volume is created with its class");
        self.calls.lock().expect("lock").push("create");
        Ok(())
    }

    fn repair(&self, volume: VolumeId, _file: &Path) -> RepairReceipt {
        self.calls.lock().expect("lock").push("repair");
        RepairReceipt::new(volume, RepairOutcome::Clean, 0, 0)
    }
}

/// A double whose creation always refuses with the same error.
struct Refusing {
    error: CreateError,
}

impl VolumeRepair for Refusing {
    fn create(&self, _volume: VolumeId, _file: &Path, _size_bytes: u64) -> Result<(), CreateError> {
        Err(self.error)
    }

    fn repair(&self, volume: VolumeId, _file: &Path) -> RepairReceipt {
        RepairReceipt::new(volume, RepairOutcome::Clean, 0, 0)
    }
}

fn with_repairs(
    directory: &tempfile::TempDir,
    repairs: Box<dyn VolumeRepair + Send + Sync>,
) -> DirectoryVolumes {
    DirectoryVolumes::open_uncertified(
        &directory.path().join("volumes"),
        &directory.path().join("records"),
        repairs,
        true,
    )
    .expect("opens")
}

#[test]
fn a_created_volume_is_made_and_then_repaired_and_an_existing_one_is_not_made_again() {
    let directory = tempfile::tempdir().expect("temp dir");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let volumes = with_repairs(
        &directory,
        Box::new(Recording {
            calls: Arc::clone(&calls),
        }),
    );
    volumes.attach(&attach(1, 1, 1, 10)).expect("attach");
    assert_eq!(
        *calls.lock().expect("lock"),
        vec!["create", "repair"],
        "a volume that does not exist yet is created before it is repaired"
    );

    calls.lock().expect("lock").clear();
    volumes.attach(&attach(1, 1, 2, 10)).expect("re-attach");
    assert_eq!(
        *calls.lock().expect("lock"),
        vec!["repair"],
        "a volume that exists is repaired and never created again"
    );
}

#[test]
fn a_refused_create_is_a_typed_refusal_and_leaves_no_attached_volume() {
    for (error, expected) in [
        (
            CreateError::Occupied,
            VolumeError::Busy("the volume path already holds data"),
        ),
        (
            CreateError::Size(0),
            VolumeError::Geometry("the volume class has no size"),
        ),
        (
            CreateError::Tool(1),
            VolumeError::FsckFailed("the volume filesystem could not be created"),
        ),
    ] {
        let directory = tempfile::tempdir().expect("temp dir");
        let volumes = with_repairs(&directory, Box::new(Refusing { error }));
        assert_eq!(volumes.attach(&attach(1, 1, 1, 10)).err(), Some(expected));
        assert_eq!(
            volumes.inspect(volume(1)).err(),
            Some(VolumeError::Missing("no sidecar names this volume")),
            "a volume that could not be created is not attached"
        );
    }
}

mod encryption;
