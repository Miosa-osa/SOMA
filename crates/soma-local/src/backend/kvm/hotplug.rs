//! Where a Generation's resizable memory region sits, from the Generation's own declaration.
//!
//! A machine has the memory device only when its Generation declared one, and the region's size
//! is part of that declaration. Both boot paths and the prepared pool read it here so they place
//! the region at the same address: above the machine's RAM, from the size alone.

use soma_generation::ProfileLimits;
use soma_kvm::DeviceSet;
use soma_kvm::memory_layout::{GuestLayout, HOTPLUG_ALIGN_BYTES, HotplugRegion};

use super::prepared::PreparedGeneration;

/// The region size in bytes the Generation declared, or `None` when it declared no region.
///
/// The size is not a field of its own: a machine contract 3 Generation binds the compiler policy
/// that built it, the policy names the largest RAM it admits, and the region is whatever that
/// ceiling leaves above the boot memory the shape binds. Every input is already in the manifest,
/// so every contract 1 and 2 manifest reads exactly as before.
pub(super) fn declared_bytes(prepared: &PreparedGeneration) -> Option<u64> {
    let manifest = &prepared.manifest;
    bytes_for(
        manifest.machine_contract.version,
        manifest.compiler_policy_version,
        manifest.shape.memory_bytes,
    )
}

/// The region a machine of `boot_bytes` under one contract and policy has above its RAM.
///
/// Rounded down to the placement alignment, because a region the host could not place would
/// otherwise turn a valid Generation into a launch failure. `None` for a contract without the
/// device, a policy with no limits, and a machine already at its ceiling.
pub(super) fn bytes_for(
    contract_version: u16,
    policy_version: u16,
    boot_bytes: u64,
) -> Option<u64> {
    if contract_version < 3 {
        return None;
    }
    let ceiling = ProfileLimits::for_version(policy_version)?
        .max_memory_mib
        .saturating_mul(1 << 20);
    let room = ceiling.checked_sub(boot_bytes)?;
    let aligned = room - room % HOTPLUG_ALIGN_BYTES;
    (aligned > 0).then_some(aligned)
}

/// Places the declared region above the machine's RAM.
///
/// A set that declares the memory device must also carry a size, and a size without the device
/// is meaningless; either disagreement is refused rather than resolved by preferring one.
///
/// # Errors
///
/// Returns `Err(())` when the declaration is inconsistent or the size cannot be placed.
pub(super) fn region_for(
    devices: DeviceSet,
    memory_bytes: u64,
    declared_bytes: Option<u64>,
) -> Result<Option<HotplugRegion>, ()> {
    match (devices.mem(), declared_bytes) {
        (false, None) => Ok(None),
        (true, Some(bytes)) => GuestLayout::new(memory_bytes)
            .ok()
            .and_then(|layout| HotplugRegion::new(layout, bytes).ok())
            .map(Some)
            .ok_or(()),
        _ => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAM: u64 = 1 << 30;

    #[test]
    fn a_generation_without_the_device_has_no_region() {
        assert_eq!(region_for(DeviceSet::FULL, RAM, None), Ok(None));
    }

    #[test]
    fn a_declared_region_is_placed_above_the_ram_and_is_stable() {
        let set = DeviceSet::FULL_WITH_MEM;
        let bytes = 16 * 128 * (1 << 20);
        let region = region_for(set, RAM, Some(bytes)).unwrap().unwrap();
        assert_eq!(region.size, bytes);
        assert!(region.guest_start >= 1 << 32);
        assert_eq!(region_for(set, RAM, Some(bytes)), Ok(Some(region)));
    }

    #[test]
    fn a_device_without_a_size_or_a_size_without_the_device_is_refused() {
        assert_eq!(region_for(DeviceSet::FULL_WITH_MEM, RAM, None), Err(()));
        assert_eq!(region_for(DeviceSet::FULL, RAM, Some(1 << 30)), Err(()));
        assert_eq!(region_for(DeviceSet::FULL_WITH_MEM, RAM, Some(1)), Err(()));
    }

    const GIB: u64 = 1 << 30;

    #[test]
    fn only_a_contract_three_generation_has_a_region_and_it_is_what_the_ceiling_leaves() {
        // Contracts 1 and 2 never had the device, whatever their policy admits.
        assert_eq!(bytes_for(1, 1, GIB), None);
        assert_eq!(bytes_for(2, 2, GIB), None);
        // Contract 3 under policy 3: the 16 GiB ceiling minus the boot memory.
        assert_eq!(bytes_for(3, 3, GIB), Some(15 * GIB));
        // A boot size that does not leave a whole alignment unit rounds the region down.
        let region = bytes_for(3, 3, GIB + (1 << 20)).unwrap();
        assert_eq!(region % HOTPLUG_ALIGN_BYTES, 0);
        assert_eq!(region, 15 * GIB - HOTPLUG_ALIGN_BYTES);
        // A machine at its ceiling has no room, and an unknown policy has no ceiling.
        assert_eq!(bytes_for(3, 3, 16 * GIB), None);
        assert_eq!(bytes_for(3, 9, GIB), None);
    }

    #[test]
    fn a_version_three_generation_gets_a_placed_region_and_an_older_one_none() {
        let boot = 2 * GIB;
        let v3 = region_for(DeviceSet::FULL_WITH_MEM, boot, bytes_for(3, 3, boot))
            .unwrap()
            .expect("a region");
        assert_eq!(v3.size, 14 * GIB);
        assert!(v3.guest_start >= 1 << 32);
        // Older Generations: no device, no size, no region, and nothing else changes.
        assert_eq!(
            region_for(DeviceSet::new(true, true), boot, bytes_for(2, 2, boot)),
            Ok(None)
        );
    }
}
