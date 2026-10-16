//! How the session builds its machine, and what one finished run's evidence must show.

use super::*;

/// The shape and contract one session's machine is built at.
#[derive(Clone, Copy)]
pub struct Machine {
    /// Guest RAM in bytes.
    pub ram_bytes: u64,
    /// The vCPU count.
    pub vcpus: u16,
    /// The machine contract the Generation was built under.
    pub contract: MachineContract,
}

/// The machine this session runs, with exactly the manifest's declared devices.
pub fn config(
    kernel: File,
    initramfs: File,
    root: File,
    overlay: File,
    machine: Machine,
    devices: DeviceSet,
) -> SandboxConfig {
    let Machine {
        ram_bytes,
        vcpus,
        contract,
    } = machine;
    let overlay = Some(overlay);
    SandboxConfig {
        kernel,
        initramfs,
        disks: SandboxDisks {
            root,
            overlay,
            volume: None,
            volume_placeholder: false,
        },
        identity: DeviceIdentity {
            guest_cid: GUEST_CID,
            guest_mac: GUEST_MAC,
        },
        ram_bytes,
        vcpus,
        contract,
        devices,
        hotplug: None,
    }
}

/// The assertions every successful sandbox run must satisfy.
pub fn assert_orderly(evidence: &SandboxEvidence) {
    assert_eq!(
        evidence.exit,
        Ok(GuestExit::Reset),
        "guest did not stop orderly"
    );
    assert!(evidence.launch_page_retired, "launch page was not retired");
    for milestone in [
        Milestone::RunStart,
        Milestone::KernelInit,
        Milestone::LaunchPageConsumed,
        Milestone::VsockConnected,
        Milestone::Handshake,
        Milestone::LaunchPageRetired,
        Milestone::Ready,
        Milestone::AgentReadyLine,
        Milestone::Execute,
        Milestone::Shutdown,
        Milestone::GuestExit,
        Milestone::Cleanup,
    ] {
        assert!(
            evidence.at(milestone).is_some(),
            "milestone {milestone:?} missing"
        );
    }
    assert!(
        evidence.devices.first_fault.is_none(),
        "a device faulted: {:?}",
        evidence.devices
    );
    assert_eq!(evidence.mmio.transport_violations, 0, "{:?}", evidence.mmio);
    assert_eq!(evidence.mmio.notify_exits, 0, "{:?}", evidence.mmio);
    evidence::assert_ports_are_expected(evidence.vcpus.len(), &evidence.bus);
    let text = String::from_utf8_lossy(&evidence.serial);
    assert!(text.contains("soma-guest-agent: ready"));
    assert!(text.contains("soma-guest-agent: shutdown acknowledged"));
    assert!(!text.contains("poisoned by"));
}
