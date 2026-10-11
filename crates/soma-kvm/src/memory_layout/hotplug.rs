//! The pure geometry of the hotplug memory region.
//!
//! A machine built with a ceiling above its boot memory owns one extra guest-physical range that
//! starts empty and that the guest grows and shrinks at run time through the memory device. The
//! range sits above the highest boot RAM address, on a 128 MiB boundary, so its place follows
//! from the boot size alone and a restore re-derives it. It is never part of the memory object:
//! the object a snapshot names is boot RAM only, which is why a capture needs nothing plugged and
//! a restore starts the region empty.

use std::fmt;

use super::{GuestLayout, MMIO_HOLE_END, PAGE_SIZE, RamRegion};

/// The smallest block the memory device admits, and so the unit a region size is a multiple of.
pub const HOTPLUG_MIN_BLOCK_BYTES: u64 = 2 * 1024 * 1024;
/// The boundary the region's first byte sits on.
pub const HOTPLUG_ALIGN_BYTES: u64 = 128 * 1024 * 1024;
/// Largest region a machine can declare.
pub const MAX_HOTPLUG_BYTES: u64 = 64 * 1024 * 1024 * 1024;
/// KVM memory slot of the region: slot 0 is low RAM, 1 the launch page, 2 high RAM.
pub const HOTPLUG_RAM_SLOT: u32 = 3;

/// Why a region cannot be laid out.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotplugError {
    /// The size is zero, which means the machine has no region.
    Empty,
    /// The size is not a whole number of minimum blocks.
    NotBlockAligned { bytes: u64 },
    /// The size is above [`MAX_HOTPLUG_BYTES`].
    AboveMaximum { bytes: u64 },
    /// The start is not where a region of this machine is placed.
    BadStart { guest_start: u64 },
}

impl fmt::Display for HotplugError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("hotplug region size must be non-zero"),
            Self::NotBlockAligned { bytes } => write!(
                formatter,
                "hotplug region size {bytes} is not a multiple of {HOTPLUG_MIN_BLOCK_BYTES}"
            ),
            Self::AboveMaximum { bytes } => write!(
                formatter,
                "hotplug region size {bytes} is above the {MAX_HOTPLUG_BYTES}-byte maximum"
            ),
            Self::BadStart { guest_start } => write!(
                formatter,
                "hotplug region start {guest_start:#x} is not a valid placement"
            ),
        }
    }
}

impl std::error::Error for HotplugError {}

/// One hotplug region: its KVM slot, where the guest sees it, and how big it is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotplugRegion {
    /// KVM memory slot number.
    pub slot: u32,
    /// First guest-physical byte.
    pub guest_start: u64,
    /// Length in bytes.
    pub size: u64,
}

impl HotplugRegion {
    /// Places a region of `size` bytes above the RAM of `layout`.
    ///
    /// The start is the first 128 MiB boundary at or above the highest boot RAM byte, and never
    /// below four gigabytes: a machine whose RAM ends under the MMIO hole puts the region above
    /// the hole rather than inside it.
    ///
    /// # Errors
    ///
    /// Returns [`HotplugError`] for a zero, unaligned, or oversized region.
    pub const fn new(layout: GuestLayout, size: u64) -> Result<Self, HotplugError> {
        let floor = if layout.guest_end() > MMIO_HOLE_END {
            layout.guest_end()
        } else {
            MMIO_HOLE_END
        };
        Self::from_parts(floor.next_multiple_of(HOTPLUG_ALIGN_BYTES), size)
    }

    /// Validates a region read back from a snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`HotplugError`] for a size or start no machine would produce.
    pub const fn from_parts(guest_start: u64, size: u64) -> Result<Self, HotplugError> {
        if size == 0 {
            return Err(HotplugError::Empty);
        }
        if !size.is_multiple_of(HOTPLUG_MIN_BLOCK_BYTES) {
            return Err(HotplugError::NotBlockAligned { bytes: size });
        }
        if size > MAX_HOTPLUG_BYTES {
            return Err(HotplugError::AboveMaximum { bytes: size });
        }
        if guest_start < MMIO_HOLE_END
            || !guest_start.is_multiple_of(HOTPLUG_ALIGN_BYTES)
            || guest_start.checked_add(size).is_none()
        {
            return Err(HotplugError::BadStart { guest_start });
        }
        Ok(Self {
            slot: HOTPLUG_RAM_SLOT,
            guest_start,
            size,
        })
    }

    /// One past the last guest-physical byte.
    #[must_use]
    pub const fn guest_end(&self) -> u64 {
        self.guest_start + self.size
    }

    /// Whether `[address, address + length)` lies inside the region.
    #[must_use]
    pub const fn contains(&self, address: u64, length: u64) -> bool {
        match address.checked_add(length) {
            Some(end) => address >= self.guest_start && end <= self.guest_end(),
            None => false,
        }
    }

    /// The offset into the region of one guest-physical byte.
    #[must_use]
    pub const fn offset_of(&self, address: u64) -> Option<u64> {
        if address >= self.guest_start && address < self.guest_end() {
            Some(address - self.guest_start)
        } else {
            None
        }
    }

    /// The range as a KVM slot over a mapping of its own, so its object offset is zero.
    #[must_use]
    pub const fn ram_region(&self) -> RamRegion {
        RamRegion {
            slot: self.slot,
            guest_start: self.guest_start,
            host_offset: 0,
            size: self.size,
        }
    }
}

const _: () = {
    assert!(HOTPLUG_ALIGN_BYTES.is_multiple_of(HOTPLUG_MIN_BLOCK_BYTES));
    assert!(HOTPLUG_MIN_BLOCK_BYTES.is_multiple_of(PAGE_SIZE));
    assert!(MAX_HOTPLUG_BYTES.is_multiple_of(HOTPLUG_MIN_BLOCK_BYTES));
    assert!(HOTPLUG_RAM_SLOT > super::HIGH_RAM_SLOT);
};

#[cfg(test)]
mod tests {
    use super::super::{LOW_RAM_END, MAX_RAM_BYTES, MIN_RAM_BYTES};
    use super::*;

    fn layout(bytes: u64) -> GuestLayout {
        GuestLayout::new(bytes).unwrap()
    }

    #[test]
    fn accepts_whole_blocks_and_rejects_the_rest() {
        let layout = layout(MIN_RAM_BYTES);
        assert!(HotplugRegion::new(layout, HOTPLUG_MIN_BLOCK_BYTES).is_ok());
        assert!(HotplugRegion::new(layout, MAX_HOTPLUG_BYTES).is_ok());
        assert_eq!(HotplugRegion::new(layout, 0), Err(HotplugError::Empty));
        assert!(matches!(
            HotplugRegion::new(layout, HOTPLUG_MIN_BLOCK_BYTES + PAGE_SIZE),
            Err(HotplugError::NotBlockAligned { .. })
        ));
        assert!(matches!(
            HotplugRegion::new(layout, MAX_HOTPLUG_BYTES + HOTPLUG_MIN_BLOCK_BYTES),
            Err(HotplugError::AboveMaximum { .. })
        ));
    }

    #[test]
    fn a_machine_without_high_ram_puts_the_region_at_four_gigabytes() {
        for ram in [MIN_RAM_BYTES, LOW_RAM_END] {
            let region = HotplugRegion::new(layout(ram), 4 * HOTPLUG_ALIGN_BYTES).unwrap();
            assert_eq!(region.guest_start, MMIO_HOLE_END);
            assert_eq!(region.slot, HOTPLUG_RAM_SLOT);
            assert_eq!(region.size, 4 * HOTPLUG_ALIGN_BYTES);
        }
    }

    #[test]
    fn a_machine_with_high_ram_puts_the_region_above_it_on_a_128_mib_boundary() {
        let layout = layout(LOW_RAM_END + 1024 * 1024 * 1024 + HOTPLUG_MIN_BLOCK_BYTES);
        let region = HotplugRegion::new(layout, HOTPLUG_ALIGN_BYTES).unwrap();
        assert!(region.guest_start >= layout.guest_end());
        assert!(region.guest_start - layout.guest_end() < HOTPLUG_ALIGN_BYTES);
        assert!(region.guest_start.is_multiple_of(HOTPLUG_ALIGN_BYTES));
        // An already aligned end is used as is.
        let aligned =
            super::super::GuestLayout::new(LOW_RAM_END + 2 * HOTPLUG_ALIGN_BYTES).unwrap();
        let region = HotplugRegion::new(aligned, HOTPLUG_ALIGN_BYTES).unwrap();
        assert_eq!(region.guest_start, aligned.guest_end());
    }

    #[test]
    fn the_region_never_overlaps_boot_ram_at_any_size() {
        for ram in [
            MIN_RAM_BYTES,
            LOW_RAM_END,
            LOW_RAM_END + PAGE_SIZE,
            MAX_RAM_BYTES,
        ] {
            let layout = layout(ram);
            let region = HotplugRegion::new(layout, HOTPLUG_ALIGN_BYTES).unwrap();
            assert!(region.guest_start >= layout.guest_end());
            assert_eq!(layout.host_offset(region.guest_start), None);
            assert!(!layout.contains(region.guest_start, 1));
        }
    }

    #[test]
    fn from_parts_refuses_placements_no_machine_produces() {
        let good = HotplugRegion::new(layout(MIN_RAM_BYTES), HOTPLUG_ALIGN_BYTES).unwrap();
        assert_eq!(
            HotplugRegion::from_parts(good.guest_start, good.size),
            Ok(good)
        );
        for start in [0, LOW_RAM_END, MMIO_HOLE_END + PAGE_SIZE, u64::MAX - 1] {
            assert!(matches!(
                HotplugRegion::from_parts(start, HOTPLUG_ALIGN_BYTES),
                Err(HotplugError::BadStart { .. })
            ));
        }
    }

    #[test]
    fn containment_and_offsets_are_checked() {
        let region = HotplugRegion::new(layout(MIN_RAM_BYTES), HOTPLUG_ALIGN_BYTES).unwrap();
        let start = region.guest_start;
        assert!(region.contains(start, HOTPLUG_ALIGN_BYTES));
        assert!(!region.contains(start, HOTPLUG_ALIGN_BYTES + 1));
        assert!(!region.contains(start - 1, 1));
        assert!(!region.contains(u64::MAX, 1));
        assert_eq!(region.offset_of(start + 5), Some(5));
        assert_eq!(region.offset_of(region.guest_end()), None);
        assert_eq!(region.offset_of(start - 1), None);
        let slot = region.ram_region();
        assert_eq!(
            (slot.slot, slot.guest_start, slot.host_offset, slot.size),
            (HOTPLUG_RAM_SLOT, start, 0, HOTPLUG_ALIGN_BYTES)
        );
    }
}
