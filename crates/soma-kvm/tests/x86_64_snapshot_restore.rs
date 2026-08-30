//! Live `x86_64` snapshot proof: one `node:22` Generation is booted to its disconnected
//! repair point, captured with no launch material anywhere in it, and restored repeatedly into
//! independent authenticated Instances.
//!
//! Run on an `x86_64` Linux host with readable and writable `/dev/kvm`, the pinned kernel, the
//! pinned erofs-utils, the built static guest agent, and a `node:22` OCI layout:
//!
//! ```sh
//! SOMA_X86_64_VMLINUX=/path/to/vmlinux-<ver>-soma-v1 \
//! SOMA_EROFS_TOOLS=/path/to/erofs-utils-1.9.4 \
//! SOMA_GUEST_AGENT=target/x86_64-unknown-linux-musl/release/soma-guest-agent \
//! SOMA_OCI_NODE_LAYOUT=/path/to/oci-node22 \
//!   cargo test --locked -p soma-kvm --test x86_64_snapshot_restore -- --ignored --test-threads=1
//! ```
//!
//! Every test shares one compiled Generation and one captured snapshot, so the suite must run
//! single-threaded in one process.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[allow(dead_code)]
#[path = "x86_64_kernel_boot/discover.rs"]
mod x86_64_discover;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[allow(dead_code)]
#[path = "x86_64_sandbox_boot/generation.rs"]
mod x86_64_sandbox_boot_generation;

#[path = "x86_64_sandbox_boot/generation_cache.rs"]
mod x86_64_sandbox_boot_generation_cache;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[allow(dead_code)]
#[path = "x86_64_sandbox_boot/control.rs"]
mod x86_64_sandbox_boot_control;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[allow(dead_code)]
#[path = "x86_64_sandbox_boot/session.rs"]
mod x86_64_sandbox_boot_session;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[allow(dead_code)]
#[path = "x86_64_sandbox_boot/host.rs"]
mod x86_64_sandbox_boot_host;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "x86_64_snapshot_restore/fixture.rs"]
mod x86_64_snapshot_restore_fixture;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "x86_64_snapshot_restore/instance.rs"]
mod x86_64_snapshot_restore_instance;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "x86_64_snapshot_restore/report.rs"]
mod x86_64_snapshot_restore_report;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "x86_64_snapshot_restore/rejection.rs"]
mod x86_64_snapshot_restore_rejection;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "x86_64_snapshot_restore/timing.rs"]
mod x86_64_snapshot_restore_timing;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod live {
    use soma_guest::TerminalStatus;
    use soma_kvm::x86_64::{GuestExit, Milestone};

    use crate::{
        x86_64_sandbox_boot_host::require_kvm, x86_64_snapshot_restore_fixture as fixture,
        x86_64_snapshot_restore_instance as instance, x86_64_snapshot_restore_report as report,
    };

    /// The path one Instance creates on its private root so another can look for it.
    const PRIVATE_MARKER: &[u8] = b"/soma-first-instance";

    #[test]
    #[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
    fn one_restore_reaches_ready_and_reports_the_node_version() {
        require_kvm();
        let fixture = fixture::shared();
        let commands = [instance::command(b"/usr/local/bin/node", &[b"--version"])];
        let restored = instance::run(&fixture, "single", 4, &commands);
        report::timeline("single", &restored.evidence);
        eprintln!(
            "[single] the restore call took {} ns before the launch page was written",
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
                "milestone {milestone:?} missing from a restored Instance"
            );
        }
        assert!(
            restored.evidence.devices.first_fault.is_none(),
            "a device faulted: {:?}",
            restored.evidence.devices
        );
        assert_eq!(restored.evidence.mmio.transport_violations, 0);
        // The launch-page slot belongs before KVM_CREATE_VCPU; see the loop test for why.
        assert!(
            restored.evidence.at(Milestone::LaunchPageMapped)
                < restored.evidence.at(Milestone::Vcpu),
            "the launch-page slot was registered after the vCPU existed"
        );
        assert_eq!(
            restored.descriptors.1, restored.descriptors.0,
            "the restored machine leaked descriptors"
        );
        assert_eq!(
            restored.threads.1, restored.threads.0,
            "the restored machine leaked threads"
        );
        let executed = &restored.executed[0];
        assert_eq!(executed.status, TerminalStatus::Exited(0));
        let stdout = String::from_utf8_lossy(&executed.stdout);
        assert!(stdout.starts_with("v22."), "stdout={stdout:?}");
        assert!(executed.stderr.is_empty());
        // A restored machine never replays the cold boot: the kernel that hands control to
        // `/init` ran once, in the machine the snapshot was taken from.
        let serial = String::from_utf8_lossy(&restored.evidence.serial);
        assert!(
            !serial.contains("Run /init as init process"),
            "the restored machine re-ran early init"
        );
        assert!(serial.contains("soma-guest-agent: ready"));
    }

    #[test]
    #[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
    fn two_restores_of_one_snapshot_are_independent_instances() {
        require_kvm();
        let fixture = fixture::shared();
        let before = report::digest(&fixture.paths.memory());
        let first = instance::run(
            &fixture,
            "clone-a",
            5,
            &[
                instance::command(b"/bin/cat", &[b"/etc/machine-id"]),
                instance::command(b"/bin/cat", &[b"/proc/sys/kernel/hostname"]),
                instance::command(b"/bin/mkdir", &[PRIVATE_MARKER]),
                instance::command(b"/bin/ls", &[b"-d", PRIVATE_MARKER]),
            ],
        );
        let second = instance::run(
            &fixture,
            "clone-b",
            6,
            &[
                instance::command(b"/bin/cat", &[b"/etc/machine-id"]),
                instance::command(b"/bin/cat", &[b"/proc/sys/kernel/hostname"]),
                instance::command(b"/bin/ls", &[b"-d", PRIVATE_MARKER]),
                instance::command(b"/usr/local/bin/node", &[b"--version"]),
            ],
        );
        report::timeline("clone-a", &first.evidence);
        report::timeline("clone-b", &second.evidence);

        assert_ne!(first.identity, second.identity, "the same InstanceId");
        assert_ne!(first.facts.guest_cid, second.facts.guest_cid);
        assert_ne!(
            u64::from(first.facts.guest_cid),
            first.facts.captured_cid,
            "the captured context identifier was reused"
        );
        // Reaching `Ready` at all proves the guest kernel reported the assigned identifier:
        // the agent refuses to connect while its own vsock device disagrees with the launch
        // page, so two Instances that both became ready held two different identifiers.
        let machine_ids = (text(&first, 0), text(&second, 0));
        let hostnames = (text(&first, 1), text(&second, 1));
        eprintln!("[clones] machine_ids={machine_ids:?} hostnames={hostnames:?}");
        assert_ne!(machine_ids.0, machine_ids.1, "the same machine identity");
        assert_ne!(hostnames.0, hostnames.1, "the same hostname");
        assert_eq!(machine_ids.0.len(), 32, "machine-id is not 32 hex digits");
        assert!(hostnames.0.starts_with("soma-"));

        assert_eq!(
            first.executed[3].status,
            TerminalStatus::Exited(0),
            "the first Instance could not see its own private write"
        );
        assert_ne!(
            second.executed[2].status,
            TerminalStatus::Exited(0),
            "the second Instance saw the first Instance's private write"
        );
        assert!(
            String::from_utf8_lossy(&second.executed[3].stdout).starts_with("v22."),
            "the second Instance did not report the Node version"
        );

        let heads = (
            report::digest(&first.head_path),
            report::digest(&second.head_path),
        );
        assert_ne!(heads.0, heads.1, "two Instances produced the same head");
        assert_ne!(
            heads.0,
            report::digest(&fixture.paths.overlay()),
            "the first head never diverged from the sterile template"
        );
        assert_eq!(
            before,
            report::digest(&fixture.paths.memory()),
            "the memory object changed under a private mapping"
        );
    }

    fn text(instance: &instance::Instance, index: usize) -> String {
        String::from_utf8_lossy(&instance.executed[index].stdout)
            .trim()
            .to_owned()
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
#[test]
fn reports_that_the_x86_64_snapshot_proof_is_unavailable_on_this_target() {
    // Live snapshot capture and restore compile only for Linux x86_64 and are never emulated.
    assert!(std::hint::black_box(true));
}
