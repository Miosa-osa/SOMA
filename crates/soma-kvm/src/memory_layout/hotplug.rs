//! The pure geometry of the hotplug memory region.
//!
//! A machine built with a ceiling above its boot memory owns one extra guest-physical range that
//! starts empty and that the guest grows and shrinks at run time. The range sits at a fixed
//! address far above any boot RAM, so its place never depends on the boot size and a restore
//! re-derives it from the size alone. It is never part of the memory object: the object a
//! snapshot names is boot RAM only, which is why a capture needs the region empty and a restore
//! starts it empty.

use std::fmt;

use super::{MAX_RAM_BYTES, MMIO_HOLE_END, PAGE_SIZE, RamRegion};

/// The unit the guest plugs and unplugs, and the unit the host discards.
pub const HOTPLUG_BLOCK_BYTES: u64 = 2 * 1024 * 1024;
/// The unit the region size is a multiple of.
pub const HOTPLUG_SLOT_BYTES: u64 = 128 * 1024 * 1024;
/// First guest-physical byte of the region, 128 GiB, above the highest boot RAM address.
pub const HOTPLUG_REGION_START: u64 = 0x20_0000_0000;
/// Largest region a machine can declare.
pub const MAX_HOTPLUG_BYTES: u64 = 64 * 1024 * 1024 * 1024;
/// KVM memory slot of the region: slot 0 is low RAM, 1 the launch page, 2 high RAM.
pub const HOTPLUG_RAM_SLOT: u32 = 3;

/// Why a region size cannot be laid out.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotplugError {
    /// The size is zero, which means the machine has no region.
    Empty,
    /// The size is not a whole number of slots.
    NotSlotAligned { bytes: u64 },
    /// The size is above [`MAX_HOTPLUG_BYTES`].
    AboveMaximum { bytes: u64 },
    /// A block range is unaligned, empty, or leaves the region.
    BadBlockRange { address: u64, length: u64 },
}

impl fmt::Display for HotplugError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("hotplug region size must be non-zero"),
            Self::NotSlotAligned { bytes } => write!(
                formatter,
                "hotplug region size {bytes} is not a multiple of {HOTPLUG_SLOT_BYTES}"
            ),
            Self::AboveMaximum { bytes } => write!(
                formatter,
                "hotplug region size {bytes} is above the {MAX_HOTPLUG_BYTES}-byte maximum"
            ),
            Self::BadBlockRange { address, length } => write!(
                formatter,
                "hotplug range {address:#x}+{length:#x} is not a whole in-region block range"
            ),
        }
    }
}

impl std::error::Error for HotplugError {}

/// A validated region size and the range it produces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotplugGeometry {
    size: u64,
}

impl HotplugGeometry {
    /// Validates `size`, the ceiling minus the boot memory.
    ///
    /// # Errors
    ///
    /// Returns [`HotplugError`] for a zero, unaligned, or oversized region.
    pub const fn new(size: u64) -> Result<Self, HotplugError> {
        if size == 0 {
            return Err(HotplugError::Empty);
        }
        if !size.is_multiple_of(HOTPLUG_SLOT_BYTES) {
            return Err(HotplugError::NotSlotAligned { bytes: size });
        }
        if size > MAX_HOTPLUG_BYTES {
            return Err(HotplugError::AboveMaximum { bytes: size });
        }
        Ok(Self { size })
    }

    /// Region length in bytes.
    #[must_use]
    pub const fn size(self) -> u64 {
        self.size
    }

    /// First guest-physical byte.
    #[must_use]
    pub const fn guest_start(self) -> u64 {
        HOTPLUG_REGION_START
    }

    /// One past the last guest-physical byte.
    #[must_use]
    pub const fn guest_end(self) -> u64 {
        HOTPLUG_REGION_START + self.size
    }

    /// Number of 2 MiB blocks.
    #[must_use]
    pub const fn block_count(self) -> u64 {
        self.size / HOTPLUG_BLOCK_BYTES
    }

    /// The range as a KVM slot over a mapping of its own, so its object offset is zero.
    #[must_use]
    pub const fn region(self) -> RamRegion {
        RamRegion {
            slot: HOTPLUG_RAM_SLOT,
            guest_start: HOTPLUG_REGION_START,
            host_offset: 0,
            size: self.size,
        }
    }

    /// Whether `[address, address + length)` lies inside the region.
    #[must_use]
    pub const fn contains(self, address: u64, length: u64) -> bool {
        match address.checked_add(length) {
            Some(end) => address >= HOTPLUG_REGION_START && end <= self.guest_end(),
            None => false,
        }
    }

    /// The offset into the region's own mapping of one guest-physical byte.
    #[must_use]
    pub const fn offset_of(self, address: u64) -> Option<u64> {
        if address >= HOTPLUG_REGION_START && address < self.guest_end() {
            Some(address - HOTPLUG_REGION_START)
        } else {
            None
        }
    }

    /// The block indices `[first, first + count)` a whole-block guest range names.
    ///
    /// # Errors
    ///
    /// Returns [`HotplugError::BadBlockRange`] when the range is empty, not block aligned, or
    /// leaves the region.
    pub const fn blocks(self, address: u64, length: u64) -> Result<(u64, u64), HotplugError> {
        let bad = HotplugError::BadBlockRange { address, length };
        if length == 0
            || !address.is_multiple_of(HOTPLUG_BLOCK_BYTES)
            || !length.is_multiple_of(HOTPLUG_BLOCK_BYTES)
            || !self.contains(address, length)
        {
            return Err(bad);
        }
        Ok((
            (address - HOTPLUG_REGION_START) / HOTPLUG_BLOCK_BYTES,
            length / HOTPLUG_BLOCK_BYTES,
        ))
    }
}

// The region must sit above every boot RAM address and on block and page boundaries, and its
// slot must not collide with the slots boot RAM and the launch page already use.
const _: () = {
    assert!(HOTPLUG_REGION_START > MMIO_HOLE_END + MAX_RAM_BYTES);
    assert!(HOTPLUG_REGION_START.is_multiple_of(HOTPLUG_BLOCK_BYTES));
    assert!(HOTPLUG_BLOCK_BYTES.is_multiple_of(PAGE_SIZE));
    assert!(HOTPLUG_SLOT_BYTES.is_multiple_of(HOTPLUG_BLOCK_BYTES));
    assert!(MAX_HOTPLUG_BYTES.is_multiple_of(HOTPLUG_SLOT_BYTES));
    assert!(HOTPLUG_RAM_SLOT > super::HIGH_RAM_SLOT);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_whole_slots_and_rejects_the_rest() {
        assert!(HotplugGeometry::new(HOTPLUG_SLOT_BYTES).is_ok());
        assert!(HotplugGeometry::new(MAX_HOTPLUG_BYTES).is_ok());
        assert_eq!(HotplugGeometry::new(0), Err(HotplugError::Empty));
        assert!(matches!(
            HotplugGeometry::new(HOTPLUG_BLOCK_BYTES),
            Err(HotplugError::NotSlotAligned { .. })
        ));
        assert!(matches!(
            HotplugGeometry::new(MAX_HOTPLUG_BYTES + HOTPLUG_SLOT_BYTES),
            Err(HotplugError::AboveMaximum { .. })
        ));
    }

    #[test]
    fn the_region_is_one_slot_at_the_fixed_address() {
        let geometry = HotplugGeometry::new(2 * HOTPLUG_SLOT_BYTES).unwrap();
        let region = geometry.region();
        assert_eq!(region.slot, HOTPLUG_RAM_SLOT);
        assert_eq!(region.guest_start, HOTPLUG_REGION_START);
        assert_eq!(region.host_offset, 0);
        assert_eq!(region.size, 2 * HOTPLUG_SLOT_BYTES);
        assert_eq!(geometry.block_count(), 128);
    }

    #[test]
    fn containment_and_block_ranges_are_checked() {
        let geometry = HotplugGeometry::new(HOTPLUG_SLOT_BYTES).unwrap();
        let start = geometry.guest_start();
        assert!(geometry.contains(start, HOTPLUG_SLOT_BYTES));
        assert!(!geometry.contains(start, HOTPLUG_SLOT_BYTES + 1));
        assert!(!geometry.contains(start - 1, 1));
        assert!(!geometry.contains(u64::MAX, 1));
        assert_eq!(geometry.offset_of(start + 5), Some(5));
        assert_eq!(geometry.offset_of(geometry.guest_end()), None);
        assert_eq!(
            geometry.blocks(start + HOTPLUG_BLOCK_BYTES, 2 * HOTPLUG_BLOCK_BYTES),
            Ok((1, 2))
        );
        for (address, length) in [
            (start + 1, HOTPLUG_BLOCK_BYTES),
            (start, HOTPLUG_BLOCK_BYTES + 1),
            (start, 0),
            (start, HOTPLUG_SLOT_BYTES + HOTPLUG_BLOCK_BYTES),
            (start - HOTPLUG_BLOCK_BYTES, HOTPLUG_BLOCK_BYTES),
        ] {
            assert!(geometry.blocks(address, length).is_err());
        }
    }

    #[test]
    fn the_region_never_overlaps_boot_ram() {
        let geometry = HotplugGeometry::new(HOTPLUG_SLOT_BYTES).unwrap();
        let layout = super::super::GuestLayout::new(MAX_RAM_BYTES).unwrap();
        assert!(geometry.guest_start() >= layout.guest_end());
        assert_eq!(layout.host_offset(geometry.guest_start()), None);
    }
}
