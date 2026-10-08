//! Everything one sandbox is built from, before any of it exists.

use std::fs::File;

use super::devices::{DeviceIdentity, SandboxDisks};
use crate::contract::MachineContract;
use crate::virtio::DeviceSet;

/// Inputs for one sandbox.
pub struct SandboxConfig {
    /// The Generation's uncompressed PVH kernel.
    pub kernel: File,
    /// The Generation's `newc` initramfs.
    pub initramfs: File,
    /// The immutable root and the private overlay head.
    pub disks: SandboxDisks,
    /// Non-secret device identity.
    pub identity: DeviceIdentity,
    /// Guest RAM in bytes; a multiple of 4 KiB within the contract's admitted range.
    pub ram_bytes: u64,
    /// The vCPU count the Generation declared; one thread runs each.
    pub vcpus: u16,
    /// The machine contract the Generation was built under.
    pub contract: MachineContract,
    /// The devices this Generation declared; it must agree with `disks`.
    pub devices: DeviceSet,
}
