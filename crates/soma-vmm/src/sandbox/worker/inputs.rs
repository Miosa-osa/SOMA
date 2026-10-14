//! The shape one cold boot starts from, and what a launch carries into it.
//!
//! Nothing here resolves a path: every file arrives as an open handle, because the jailed side
//! has an empty root, no procfs, and a filter that kills `open` once it narrows.

use soma_guest::{HostLaunchMaterial, SecretFile};
use soma_kvm::x86_64::{DeviceIdentity, SandboxConfig, SandboxDisks};
use soma_kvm::{DeviceSet, MachineContract};

use super::super::identity::GUEST_MAC;

/// What one Instance's launch carries into the machine that will serve it.
///
/// The two travel together because they are consumed together and in one order: the material
/// authenticates the session, and the secrets are the first thing placed over it.
pub struct LaunchInputs<'a> {
    pub material: HostLaunchMaterial,
    pub secrets: &'a [SecretFile],
}

/// The opened artifacts and declared shape one cold boot starts from.
pub struct ColdBootInputs {
    pub kernel: std::fs::File,
    pub initramfs: std::fs::File,
    pub root: std::fs::File,
    /// The Instance-private head, or `None` for a Generation with no writable storage.
    pub overlay: Option<std::fs::File>,
    /// The Instance's durable volume file, or `None` for a Generation that declares none.
    ///
    /// A cold boot writes it from the first instruction, so it is the file rather than a
    /// placeholder: there is no swap to wait for.
    pub volume: Option<std::fs::File>,
    pub ram_bytes: u64,
    pub guest_cid: u32,
    /// The vCPU count the Generation declared; one thread runs each.
    pub vcpus: u16,
    /// The machine contract the Generation was built under.
    pub contract: MachineContract,
    pub devices: DeviceSet,
    /// The resizable region the Generation declared, placed above the machine's RAM.
    pub hotplug: Option<soma_kvm::memory_layout::HotplugRegion>,
}

/// The device identity and shape one sandbox is given.
#[must_use]
pub fn config(inputs: ColdBootInputs) -> SandboxConfig {
    let ColdBootInputs {
        kernel,
        initramfs,
        root,
        overlay,
        volume,
        ram_bytes,
        guest_cid,
        vcpus,
        contract,
        devices,
        hotplug,
    } = inputs;
    SandboxConfig {
        kernel,
        initramfs,
        disks: SandboxDisks {
            root,
            overlay,
            volume,
            // A worker is handed the Instance's own volume, never a capture's placeholder.
            volume_placeholder: false,
        },
        identity: DeviceIdentity {
            guest_cid,
            guest_mac: GUEST_MAC,
        },
        ram_bytes,
        vcpus,
        contract,
        devices,
        hotplug,
    }
}
