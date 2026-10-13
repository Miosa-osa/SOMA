use super::*;

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
    let volumes = DirectoryVolumes::open(
        &directory.path().join("volumes"),
        &directory.path().join("records"),
        Box::new(CleanRepair),
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
            DirectoryVolumes::open(&root, &records, Box::new(CleanRepair)).expect("opens");
        volumes.attach(&attach(1, 9, 1, 10)).expect("attaches");
    }
    let reopened = DirectoryVolumes::open(&root, &records, Box::new(CleanRepair)).expect("opens");
    let sidecar = reopened.inspect(volume(1)).expect("exists");
    assert_eq!(sidecar.state, VolumeState::Attached);
    assert_eq!(sidecar.tenant, "tenant-alpha");
    assert_eq!(sidecar.size_class_gib, 10);
}
