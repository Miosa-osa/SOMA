//! The cold-boot configuration one sandbox is built from.
//!
//! A cold boot is the request that pays for the kernel, so its configuration is assembled in one
//! place from the Generation's declared shape and the Instance's own disk.

use soma_kvm::DeviceSet;
use soma_kvm::x86_64::{DeviceIdentity, SandboxConfig, SandboxDisks};

use super::super::identity::GUEST_MAC;

/// The opened artifacts and declared shape one cold boot starts from.
pub struct ColdBootInputs {
    pub kernel: std::fs::File,
    pub initramfs: std::fs::File,
    pub root: std::fs::File,
    /// The Instance-private head, or `None` for a Generation with no writable storage.
    pub overlay: Option<std::fs::File>,
    pub ram_bytes: u64,
    pub guest_cid: u32,
    pub devices: DeviceSet,
    pub vcpus: u16,
    pub contract: soma_kvm::MachineContract,
}

/// The device identity and shape one sandbox is given.
#[must_use]
pub fn config(inputs: ColdBootInputs) -> SandboxConfig {
    let ColdBootInputs {
        kernel,
        initramfs,
        root,
        overlay,
        ram_bytes,
        guest_cid,
        devices,
        vcpus,
        contract,
    } = inputs;
    SandboxConfig {
        kernel,
        initramfs,
        disks: SandboxDisks { root, overlay },
        identity: DeviceIdentity {
            guest_cid,
            guest_mac: GUEST_MAC,
        },
        ram_bytes,
        vcpus,
        contract,
        devices,
    }
}
