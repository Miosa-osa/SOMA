//! A volume another process still has open is never moved to a new Instance (SOMA#231).
//!
//! This lives in its own test binary because it starts a process: a child shares the parent's
//! descriptors for a moment before it executes, which would hold another test's lock past the
//! point that test released it.

#![cfg(target_os = "linux")]

use std::fs;

use soma_hostd::{
    AttachRequest, CleanRepair, DirectoryVolumes, GenerationId, InstanceId, OperationId,
    RequestFingerprint, VolumeError, VolumeId, VolumeStore, volume::keyring::HostKeyring,
};

fn attach(instance: u8, operation: u8) -> AttachRequest {
    AttachRequest {
        operation: OperationId::new([operation; 16]).expect("nonzero"),
        instance: InstanceId::new([instance; 16]).expect("nonzero"),
        volume: VolumeId::new([1; 16]).expect("nonzero"),
        fingerprint: RequestFingerprint::new([operation; 32]).expect("nonzero"),
        size_class_gib: 10,
        generation: GenerationId::new([1; 32]).expect("nonzero"),
        tenant: "tenant-alpha".to_owned(),
    }
}

#[test]
fn a_volume_another_process_has_open_is_never_taken_over_even_without_a_lock() {
    let work = tempfile::tempdir().expect("temp dir");
    let records = work.path().join("records");
    let volumes = DirectoryVolumes::open_uncertified(
        &work.path().join("volumes"),
        &records,
        Box::new(CleanRepair),
        true,
    )
    .expect("opens")
    .with_keys(Box::new(HostKeyring::at(
        work.path().join("volume-master.key"),
        &records,
    )));
    volumes.attach(&attach(9, 1)).expect("attaches");
    let file = volumes.file(VolumeId::new([1; 16]).expect("nonzero"));
    fs::File::create(&file).expect("volume file");
    // A worker from before every launch path took the lock holds the file with no lock.
    let mut holder = std::process::Command::new("sleep")
        .arg("30")
        .stdin(fs::File::open(&file).expect("open"))
        .spawn()
        .expect("a process holding the file");
    let refused = volumes.attach(&attach(8, 2));
    let _ = holder.kill();
    let _ = holder.wait();
    assert_eq!(
        refused,
        Err(VolumeError::Busy("the volume is still in use"))
    );
    let owner = volumes
        .inspect(VolumeId::new([1; 16]).expect("nonzero"))
        .expect("exists")
        .instance;
    assert_eq!(owner, InstanceId::new([9; 16]).expect("nonzero"));

    volumes
        .attach(&attach(8, 3))
        .expect("once the process is gone the volume moves");
}
