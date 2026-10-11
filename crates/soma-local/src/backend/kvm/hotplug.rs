//! Where a Generation's resizable memory region sits, from the Generation's own declaration.
//!
//! A machine has the memory device only when its Generation declared one, and the region's size
//! is part of that declaration. Both boot paths and the prepared pool read it here so they place
//! the region at the same address: above the machine's RAM, from the size alone.

use soma_kvm::DeviceSet;
use soma_kvm::memory_layout::{GuestLayout, HotplugRegion};

use super::prepared::PreparedGeneration;

/// The region size in bytes the Generation declared, or `None` when it declared no region.
///
/// A version 3 manifest binds the size in its shape; every earlier manifest has zero there and
/// reads exactly as before.
pub(super) fn declared_bytes(prepared: &PreparedGeneration) -> Option<u64> {
    bytes_of(prepared.manifest.shape.hotplug_bytes)
}

/// The declared size, with zero meaning no region.
const fn bytes_of(hotplug_bytes: u64) -> Option<u64> {
    if hotplug_bytes == 0 {
        None
    } else {
        Some(hotplug_bytes)
    }
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
    fn a_zero_size_is_no_region_and_a_size_is_the_declaration() {
        assert_eq!(bytes_of(0), None);
        assert_eq!(bytes_of(GIB), Some(GIB));
    }

    #[test]
    fn a_version_three_generation_gets_a_placed_region_and_an_older_one_none() {
        let boot = 2 * GIB;
        let v3 = region_for(DeviceSet::FULL_WITH_MEM, boot, bytes_of(14 * GIB))
            .unwrap()
            .expect("a region");
        assert_eq!(v3.size, 14 * GIB);
        assert!(v3.guest_start >= 1 << 32);
        // Older Generations: no device, no size, no region, and nothing else changes.
        assert_eq!(
            region_for(DeviceSet::new(true, true), boot, bytes_of(0)),
            Ok(None)
        );
    }
}
