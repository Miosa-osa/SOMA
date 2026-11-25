//! A volume the store could not create leaves nothing behind, and a store without room refuses
//! a new volume as a capacity refusal.

use super::repair::Refusing;
use super::*;

fn leftovers(directory: &tempfile::TempDir, volume_id: VolumeId) -> Vec<std::path::PathBuf> {
    let root = directory.path();
    [
        root.join("volumes").join(volume_id.hex()),
        root.join("records").join(volume_id.hex()),
        root.join("records").join("keys").join(volume_id.hex()),
    ]
    .into_iter()
    .filter(|path| path.exists())
    .collect()
}

/// A create that failed after it began wrote its sidecar, its key and its records; left there,
/// the sidecar named a volume with no filesystem the next time the store opened.
#[test]
fn a_refused_create_leaves_no_sidecar_key_or_records() {
    let directory = tempfile::tempdir().expect("temp dir");
    let volumes = with_repairs(
        &directory,
        Box::new(Refusing {
            error: CreateError::Tool(1),
        }),
    );
    assert!(volumes.attach(&attach(1, 1, 1, 10)).is_err());
    assert_eq!(
        leftovers(&directory, volume(1)),
        Vec::<std::path::PathBuf>::new()
    );
    drop(volumes);
    let reopened = with_repairs(&directory, Box::new(CleanRepair));
    assert_eq!(
        reopened.inspect(volume(1)).err(),
        Some(VolumeError::Missing("no sidecar names this volume"))
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_store_below_its_reserve_refuses_a_new_volume_as_capacity_and_writes_nothing() {
    let directory = tempfile::tempdir().expect("temp dir");
    let volumes = with_repairs(&directory, Box::new(CleanRepair)).with_reserve(u64::MAX);
    assert_eq!(
        volumes.attach(&attach(1, 1, 1, 10)).err(),
        Some(VolumeError::Capacity(
            "the volume store has no room for another volume"
        ))
    );
    assert_eq!(
        leftovers(&directory, volume(1)),
        Vec::<std::path::PathBuf>::new()
    );
    assert_eq!(
        crate::protocol::volume_failure_code(&VolumeError::Capacity("full")),
        crate::protocol::FailureCode::Capacity,
        "the wire carries it as the capacity refusal soma-api answers 507"
    );
}

/// The reserve is for new volumes: one the store already holds attaches again on a full root.
#[cfg(target_os = "linux")]
#[test]
fn a_volume_the_store_holds_still_attaches_below_the_reserve() {
    let directory = tempfile::tempdir().expect("temp dir");
    with_repairs(&directory, Box::new(CleanRepair))
        .attach(&attach(1, 1, 1, 10))
        .expect("first attach");
    let full = with_repairs(&directory, Box::new(CleanRepair)).with_reserve(u64::MAX);
    full.attach(&attach(1, 1, 2, 10)).expect("re-attach");
}
