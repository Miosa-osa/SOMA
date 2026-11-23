//! The store drives the injected repair: create, then repair, and a refused create attaches nothing.

use super::*;

pub(super) struct Recording {
    pub(super) calls: Arc<Mutex<Vec<&'static str>>>,
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
