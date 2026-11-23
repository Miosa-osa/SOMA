//! Resident pause, resume, checkpoint, restore and fork of a running Linux sandbox.

use soma_guest::TerminalStatus;
use soma_kvm::x86_64::{GuestExit, Milestone};

use crate::{
    x86_64_sandbox_boot_host::require_kvm, x86_64_snapshot_restore_fixture as fixture,
    x86_64_snapshot_restore_instance as instance,
};

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn pauses_an_authenticated_linux_sandbox_after_guest_execution() {
    use crate::x86_64_snapshot_restore_workload::PauseAfterCommand;
    require_kvm();
    let fixture = fixture::shared();
    let restored = instance::run_workload(&fixture, "resident-pause", 4, PauseAfterCommand);
    let output = String::from_utf8_lossy(&restored.output.stdout);
    assert_eq!(restored.output.status, TerminalStatus::Exited(0));
    assert!(output.starts_with("v22."), "stdout={output:?}");
    assert_eq!(restored.evidence.exit, Ok(GuestExit::Paused));
    assert!(restored.evidence.at(Milestone::Ready).is_some());
    assert!(restored.evidence.at(Milestone::Shutdown).is_none());
    assert_eq!(restored.descriptors.0, restored.descriptors.1);
    assert_eq!(restored.threads.0, restored.threads.1);
    eprintln!(
        "[resident-pause] generation={} node={} exit=Paused",
        fixture.compiled.id().as_str(),
        output.trim()
    );
}

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn resumes_an_authenticated_linux_sandbox_with_process_memory() {
    use crate::x86_64_snapshot_restore_workload::ResumeResidentProcess;
    require_kvm();
    let fixture = fixture::shared();
    let restored = instance::run_workload(&fixture, "resident-resume", 4, ResumeResidentProcess);
    assert!(restored.evidence.at(Milestone::Ready).is_some());
    assert!(restored.evidence.at(Milestone::Shutdown).is_some());
    assert_eq!(restored.descriptors.0, restored.descriptors.1);
    assert_eq!(restored.threads.0, restored.threads.1);
    eprintln!(
        "[resident-resume] generation={} same process and memory, counter {} -> {}",
        fixture.compiled.id().as_str(),
        restored.output.0,
        restored.output.1,
    );
}

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn checkpoints_a_running_linux_sandbox_and_keeps_the_original_alive() {
    use crate::x86_64_snapshot_restore_workload::CheckpointResidentProcess;
    require_kvm();
    let fixture = fixture::shared();
    let restored = instance::run_workload(
        &fixture,
        "resident-checkpoint",
        4,
        CheckpointResidentProcess {
            instance_id: [0; 16],
            candidate_id: fixture.candidate_id,
            root_digest: fixture.capture.root_digest,
            contract: fixture.contract,
        },
    );
    assert!(restored.evidence.at(Milestone::Shutdown).is_some());
    assert_eq!(restored.descriptors.0, restored.descriptors.1);
    assert_eq!(restored.threads.0, restored.threads.1);
    eprintln!(
        "[resident-checkpoint] generation={} same process and memory, counter {} -> {}",
        fixture.compiled.id().as_str(),
        restored.output.0,
        restored.output.1,
    );
}

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn restores_process_execution_after_the_original_instance_shuts_down() {
    require_kvm();
    let fixture = fixture::shared();
    crate::x86_64_snapshot_restore_resident::verify(&fixture);
}

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, a fork-capable guest agent, and a node:22 OCI layout"]
fn forks_of_a_running_checkpoint_have_independent_memory_disks_and_identity() {
    require_kvm();
    let fixture = fixture::shared();
    crate::x86_64_snapshot_restore_resident::verify_fork(&fixture);
}
