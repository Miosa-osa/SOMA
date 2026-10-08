//! The machine contract v2 snapshot proof: eight processors and sixteen gigabytes, captured at
//! the repair point and restored into a live Instance.
//!
//! Its own file because the harness root is close to the repository's source ceiling, and its own
//! fixture because the node:22 machine every other test here shares is a single-vCPU one.

use soma_guest::TerminalStatus;
use soma_kvm::x86_64::{GuestExit, Milestone};

use crate::{
    x86_64_sandbox_boot_host::require_kvm, x86_64_sandbox_boot_memory_fill as fill,
    x86_64_sandbox_boot_session as session, x86_64_snapshot_restore_fixture as fixture,
    x86_64_snapshot_restore_instance as instance, x86_64_snapshot_restore_report as report,
};

/// The machine contract v2 snapshot gate.
///
/// The capture walks the whole memory object across both ranges of the split layout and reads one
/// state section per processor; the restore re-derives those ranges and installs every processor's
/// state again. `nproc` answers only if all eight came back, and the fill answers only if the
/// restored memory object still backs both ranges, because a machine that lost its high range
/// reports the memory total and faults on the first write past the hole.
#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a busybox OCI layout"]
fn an_eight_vcpu_sixteen_gigabyte_machine_captures_and_restores() {
    require_kvm();
    let fixture = fixture::shared_v2();
    // The fill script asserts the shape it was written for, so the fixture and the script have to
    // agree before either of them runs.
    assert_eq!(
        fixture.vcpus, 8,
        "the v2 fixture is not the eight-vCPU shape"
    );
    assert_eq!(
        fixture.ram_bytes,
        fill::MEMORY_MIB * 1024 * 1024,
        "the v2 fixture is not the sixteen-gigabyte shape"
    );
    let filling = session::Command {
        program: b"/bin/sh",
        arguments: &[b"-c", fill::SCRIPT.as_bytes()],
        // Writing and reading back fourteen gigabytes is memory bandwidth work, so this step gets
        // the machine's own patience: a shorter bound would read as a failed restore.
        timeout_millis: 300_000,
        output_bytes: 65_536,
    };
    let commands = [instance::command(b"/bin/busybox", &[b"nproc"]), filling];
    let restored = instance::run(&fixture, "v2", 7, &commands);
    report::timeline("v2", &restored.evidence);
    eprintln!(
        "[v2] the restore call took {} ns before the launch page was written",
        restored.restore_ns
    );

    assert_eq!(restored.evidence.exit, Ok(GuestExit::Reset));
    assert!(restored.evidence.launch_page_retired);
    for milestone in [
        Milestone::ValidateManifest,
        Milestone::MapMemory,
        Milestone::LaunchPageMapped,
        Milestone::RegisterSlots,
        Milestone::Devices,
        Milestone::VcpuRestored,
        Milestone::RunStart,
        Milestone::LaunchPageConsumed,
        Milestone::Handshake,
        Milestone::LaunchPageRetired,
        Milestone::Ready,
        Milestone::Execute,
        Milestone::Cleanup,
    ] {
        assert!(
            restored.evidence.at(milestone).is_some(),
            "milestone {milestone:?} missing from a restored eight-vCPU Instance"
        );
    }
    assert!(
        restored.evidence.devices.first_fault.is_none(),
        "a device faulted: {:?}",
        restored.evidence.devices
    );
    assert_eq!(restored.evidence.mmio.transport_violations, 0);
    assert_eq!(
        restored.descriptors.1, restored.descriptors.0,
        "the restored machine leaked descriptors"
    );
    assert_eq!(
        restored.threads.1, restored.threads.0,
        "the restored machine leaked threads"
    );
    // One result per command, in order: the processor count first, then the memory report.
    assert_eq!(restored.output[0].status, TerminalStatus::Exited(0));
    assert_eq!(
        String::from_utf8_lossy(&restored.output[0].stdout).trim(),
        "8",
        "the restored machine did not come back with every processor"
    );
    assert_eq!(restored.output[1].status, TerminalStatus::Exited(0));
    let filled = String::from_utf8_lossy(&restored.output[1].stdout);
    eprintln!("[v2] the restored machine reported:\n{filled}");
    fill::assert_wrote_all_the_ram(&filled);
}
