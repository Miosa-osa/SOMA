//! A volume moves to a new Instance only once the machine that owned it is gone (SOMA#231).

use super::*;

/// SOMA#231: the machine that owned the volume is gone (it crashed, or it was detached), so an
/// attach for another Instance of the same tenant takes the volume over. Only Linux has the
/// process table the proof reads; elsewhere a takeover is refused.
#[cfg(target_os = "linux")]
#[test]
fn a_volume_whose_machine_is_gone_is_taken_over_by_a_new_instance() {
    for detached_first in [true, false] {
        let (_work, volumes) = store();
        volumes.attach(&attach(1, 9, 1, 10)).expect("attaches");
        fs::File::create(volumes.file(volume(1))).expect("volume file");
        if detached_first {
            volumes.detach(&detach(1, 9, 2)).expect("detaches");
        }
        let outcome = volumes
            .attach(&attach(1, 8, 3, 10))
            .expect("a volume nobody holds moves to the new Instance");
        assert_eq!(outcome.state, VolumeState::Attached);
        assert_eq!(
            volumes.inspect(volume(1)).expect("exists").instance,
            instance(8)
        );
        assert_eq!(volumes.owned(instance(8)).expect("owned").volume, volume(1));
        assert!(
            volumes.owned(instance(9)).is_err(),
            "the old Instance owns nothing"
        );
    }
}

#[test]
fn a_volume_whose_worker_still_holds_its_lock_is_never_taken_over() {
    let (_work, volumes) = store();
    volumes.attach(&attach(1, 9, 1, 10)).expect("attaches");
    let worker = fs::File::create(volumes.file(volume(1))).expect("volume file");
    worker.try_lock().expect("the live worker's lock");
    assert_eq!(
        volumes.attach(&attach(1, 8, 2, 10)),
        Err(VolumeError::Busy("the volume is still in use"))
    );
    assert_eq!(
        volumes.inspect(volume(1)).expect("exists").instance,
        instance(9)
    );
}

#[test]
fn a_volume_of_another_tenant_is_never_taken_over() {
    let (_work, volumes) = store();
    volumes.attach(&attach(1, 9, 1, 10)).expect("attaches");
    fs::File::create(volumes.file(volume(1))).expect("volume file");
    let mut other = attach(1, 8, 2, 10);
    other.tenant = "tenant-beta".to_owned();
    assert_eq!(
        volumes.attach(&other),
        Err(VolumeError::Busy("the volume belongs to another Instance"))
    );
}

/// SOMA#234: a start of a volume a live machine holds is refused as busy, not repaired.
#[test]
fn a_start_of_a_volume_a_live_machine_holds_is_refused_as_busy() {
    let (_work, volumes) = store();
    volumes.attach(&attach(1, 9, 1, 10)).expect("attaches");
    let worker = fs::File::create(volumes.file(volume(1))).expect("volume file");
    worker
        .try_lock()
        .expect("the first create's machine holds the volume");
    assert_eq!(
        volumes.binding(instance(9)).map(|binding| binding.volume),
        Err(VolumeError::Busy("the volume is still in use"))
    );
    drop(worker);
    volumes
        .binding(instance(9))
        .expect("once the machine is gone the start proceeds");
}
