use crate::{FailureKind, InstanceId, Machine};

use super::{
    fixtures::{execute, launch, stop},
    platform::{DeterministicPlatform, TestStage},
};

#[test]
fn only_the_owned_instance_can_pause_and_it_can_still_be_stopped() {
    let mut machine = Machine::with_platform(DeterministicPlatform::healthy());
    let instance = launch().instance_id();
    assert_eq!(
        machine.pause(instance).unwrap_err().kind(),
        FailureKind::InvalidLifecycle
    );
    machine.launch(launch()).unwrap();
    let wrong = InstanceId::new([0xaa; 16]).unwrap();
    assert_eq!(
        machine.pause(wrong).unwrap_err().kind(),
        FailureKind::InstanceMismatch
    );

    machine.pause(instance).unwrap();
    let refusal = machine.execute(execute([0x42; 16])).unwrap_err();
    assert_eq!(refusal.kind(), FailureKind::InvalidLifecycle);
    assert_eq!(
        machine.pause(wrong).unwrap_err().kind(),
        FailureKind::InstanceMismatch
    );
    machine.pause(instance).unwrap();
    machine.stop(stop([0x43; 16])).unwrap();
}

#[test]
fn a_failed_platform_pause_cannot_be_replayed_as_success() {
    let mut machine = Machine::with_platform(DeterministicPlatform::failing_at(TestStage::Pause));
    let instance = launch().instance_id();
    machine.launch(launch()).unwrap();
    assert_eq!(
        machine.pause(instance).unwrap_err().kind(),
        FailureKind::PauseFailed
    );
    assert_eq!(
        machine.pause(instance).unwrap_err().kind(),
        FailureKind::InvalidLifecycle
    );
    machine.stop(stop([0x43; 16])).unwrap();
}

#[test]
fn resume_reuses_the_owned_machine_and_rejects_a_foreign_instance() {
    let mut machine = Machine::with_platform(DeterministicPlatform::healthy());
    let instance = launch().instance_id();
    machine.launch(launch()).unwrap();
    machine.pause(instance).unwrap();
    let wrong = InstanceId::new([0xaa; 16]).unwrap();
    assert_eq!(
        machine.resume(wrong).unwrap_err().kind(),
        FailureKind::InstanceMismatch
    );
    machine.resume(instance).unwrap();
    machine.resume(instance).unwrap();
    machine.execute(execute([0x44; 16])).unwrap();
    machine.stop(stop([0x45; 16])).unwrap();
}

#[test]
fn failed_resume_stays_reclaimable_and_is_not_replayed_as_success() {
    let mut machine = Machine::with_platform(DeterministicPlatform::failing_at(TestStage::Resume));
    let instance = launch().instance_id();
    machine.launch(launch()).unwrap();
    machine.pause(instance).unwrap();
    assert_eq!(
        machine.resume(instance).unwrap_err().kind(),
        FailureKind::ResumeFailed
    );
    assert_eq!(
        machine.resume(instance).unwrap_err().kind(),
        FailureKind::InvalidLifecycle
    );
    machine.stop(stop([0x43; 16])).unwrap();
}
