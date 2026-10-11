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
/// This is the single place the Generation's manifest is read for the size, so the machine
/// contract version that introduces the declaration changes this function and nothing else.
pub(super) fn declared_bytes(prepared: &PreparedGeneration) -> Option<u64> {
    ceiling_region(
        prepared.manifest.machine_contract.version,
        prepared.manifest.shape.memory_bytes,
    )
}

/// The bytes between a machine's boot memory and its contract's ceiling, in whole slots.
///
/// Machine contract 3 is the one with the memory device, and it declares the region as all the
/// room the contract leaves above what the Generation boots with, so a Generation never has to
/// restate a ceiling the contract already fixes. A contract without the device, or a Generation
/// that boots at the ceiling and so leaves no whole slot, declares no region.
fn ceiling_region(contract_version: u16, boot_bytes: u64) -> Option<u64> {
    let contract = soma_kvm::MachineContract::from_version(contract_version)?;
    if !contract.has_memory_device() {
        return None;
    }
    let room = contract.max_memory_bytes().checked_sub(boot_bytes)?;
    let slots = room / SLOT_BYTES;
    (slots > 0).then(|| slots * SLOT_BYTES)
}

/// The granularity a region is declared in: one 128 MiB slot, the unit the kernel adds memory in.
const SLOT_BYTES: u64 = 128 * 1024 * 1024;

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
    fn contract_three_declares_the_room_above_the_boot_memory_and_older_ones_declare_none() {
        let gib = 1_u64 << 30;
        assert_eq!(ceiling_region(1, 2 * gib), None);
        assert_eq!(ceiling_region(2, 2 * gib), None);
        assert_eq!(ceiling_region(3, 2 * gib), Some(14 * gib));
        assert_eq!(
            ceiling_region(3, 16 * gib),
            None,
            "no room above the ceiling"
        );
        assert_eq!(
            ceiling_region(3, 16 * gib - 64 * 1024 * 1024),
            None,
            "less than one slot"
        );
        assert_eq!(
            ceiling_region(9, 2 * gib),
            None,
            "an unknown contract declares nothing"
        );
    }

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
}
