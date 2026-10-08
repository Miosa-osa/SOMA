//! The pure guest-physical memory geometry of the `x86_64` machine contract.
//!
//! Guest RAM is one contiguous range from address zero up to the MMIO boundary, and whatever
//! the contract admits beyond that boundary continues above four gigabytes. The fixed MMIO
//! window and the TSS page live in the hole between them, so a machine with more than three
//! gigabytes of RAM keeps the device addresses version 1 fixed instead of moving them.
//!
//! Everything here is arithmetic with no KVM and no architecture gate, unlike the machine under
//! [`crate::x86_64`]: the same region list, the same guest-to-object translation, and the same
//! memory map are what a client on another host verifies, so they are unit-tested there.
//!
//! The object [`GuestLayout::ram_bytes`] names is exactly the backed RAM: the hole is
//! address space, never bytes, so `memory.raw` stays the size the snapshot format documents.

use std::fmt;

use crate::contract::{MIN_MEMORY_BYTES, V2_MAX_MEMORY_BYTES};

mod region;

pub use region::{MemoryKind, MemoryMapEntry, RamRegion};

/// Guest page size.
pub const PAGE_SIZE: u64 = 4096;
/// Smallest guest RAM any contract admits.
pub const MIN_RAM_BYTES: u64 = MIN_MEMORY_BYTES;
/// Largest guest RAM any contract admits.
pub const MAX_RAM_BYTES: u64 = V2_MAX_MEMORY_BYTES;

/// First byte of the reserved legacy hole, reported reserved on every contract.
pub const LEGACY_HOLE_START: u64 = 0x000a_0000;
/// First byte above the legacy hole; high memory and the pinned kernel start here.
pub const HIGH_MEMORY_START: u64 = 0x0010_0000;
/// Physical start of the pinned kernel, equal to its `CONFIG_PHYSICAL_START`.
pub const KERNEL_START: u64 = 0x0100_0000;

/// One past the last byte of low RAM; the MMIO hole begins here.
pub const LOW_RAM_END: u64 = 0xc000_0000;
/// First byte of high RAM; the MMIO hole ends here.
pub const MMIO_HOLE_END: u64 = 0x1_0000_0000;
/// Length of the MMIO hole between the low and high RAM ranges.
pub const MMIO_HOLE_BYTES: u64 = MMIO_HOLE_END - LOW_RAM_END;

/// Largest number of RAM regions a machine can have: low, and high when RAM crosses the hole.
pub const MAX_RAM_REGIONS: usize = 2;
/// KVM memory slot of the high RAM range.
///
/// Slot 1 belongs to the dedicated launch page on every contract, so the second RAM range skips
/// it rather than displacing a slot whose number the snapshot header and the guest protocol
/// already name.
pub const HIGH_RAM_SLOT: u32 = 2;

/// Why a guest RAM size cannot be laid out.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutError {
    /// The size is not a whole number of pages.
    NotPageAligned { bytes: u64 },
    /// The size is below the contract minimum.
    BelowMinimum { bytes: u64 },
    /// The size is above the largest contract maximum.
    AboveMaximum { bytes: u64 },
}

impl fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPageAligned { bytes } => {
                write!(
                    formatter,
                    "guest RAM size {bytes} is not a multiple of {PAGE_SIZE}"
                )
            }
            Self::BelowMinimum { bytes } => write!(
                formatter,
                "guest RAM size {bytes} is below the {MIN_RAM_BYTES}-byte minimum"
            ),
            Self::AboveMaximum { bytes } => write!(
                formatter,
                "guest RAM size {bytes} is above the {MAX_RAM_BYTES}-byte maximum"
            ),
        }
    }
}

impl std::error::Error for LayoutError {}

impl LayoutError {
    /// The same reason as a static phrase, for callers whose error type carries one.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::NotPageAligned { .. } => "guest RAM size must be a multiple of 4 KiB",
            Self::BelowMinimum { .. } => "guest RAM size is below the contract minimum",
            Self::AboveMaximum { .. } => "guest RAM size is above the largest contract maximum",
        }
    }
}

/// A validated guest RAM size and the ranges and memory map it produces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GuestLayout {
    ram_bytes: u64,
    regions: [RamRegion; MAX_RAM_REGIONS],
    region_count: usize,
}

impl GuestLayout {
    /// Validates `ram_bytes` and derives the range list.
    ///
    /// A size at or below [`LOW_RAM_END`] is one range from address zero, which is exactly the
    /// version 1 layout; a larger size is that same low range plus one above the MMIO hole.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError::NotPageAligned`], [`LayoutError::BelowMinimum`], or
    /// [`LayoutError::AboveMaximum`] for a size the machine cannot lay out.
    pub fn new(ram_bytes: u64) -> Result<Self, LayoutError> {
        if !ram_bytes.is_multiple_of(PAGE_SIZE) {
            return Err(LayoutError::NotPageAligned { bytes: ram_bytes });
        }
        if ram_bytes < MIN_RAM_BYTES {
            return Err(LayoutError::BelowMinimum { bytes: ram_bytes });
        }
        if ram_bytes > MAX_RAM_BYTES {
            return Err(LayoutError::AboveMaximum { bytes: ram_bytes });
        }
        let low = ram_bytes.min(LOW_RAM_END);
        let mut regions = [RamRegion {
            slot: 0,
            guest_start: 0,
            host_offset: 0,
            size: low,
        }; MAX_RAM_REGIONS];
        let mut region_count = 1_usize;
        if ram_bytes > LOW_RAM_END {
            regions[1] = RamRegion {
                slot: HIGH_RAM_SLOT,
                guest_start: MMIO_HOLE_END,
                host_offset: low,
                size: ram_bytes - low,
            };
            region_count = 2;
        }
        Ok(Self {
            ram_bytes,
            regions,
            region_count,
        })
    }

    /// Total backed guest RAM, which is exactly the memory object's size.
    #[must_use]
    pub const fn ram_bytes(self) -> u64 {
        self.ram_bytes
    }

    /// The backed ranges, in ascending guest order.
    #[must_use]
    pub fn regions(&self) -> &[RamRegion] {
        &self.regions[..self.region_count]
    }

    /// Whether more than one range is backed, which is true only past [`LOW_RAM_END`].
    #[must_use]
    pub const fn is_split(self) -> bool {
        self.region_count == 2
    }

    /// One past the highest backed guest-physical address.
    #[must_use]
    pub const fn guest_end(self) -> u64 {
        self.regions[self.region_count - 1].guest_end()
    }

    /// One past the low range, which is where 32-bit boot artifacts are placed.
    ///
    /// The initramfs goes top-down below this ceiling rather than below the top of RAM, so it
    /// stays reachable through a 32-bit boot field on a machine whose RAM continues above four
    /// gigabytes. For a machine whose RAM ends at or below the MMIO boundary this is the end of
    /// RAM, which is exactly where version 1 has always placed it.
    #[must_use]
    pub const fn low_end(self) -> u64 {
        self.regions[0].guest_end()
    }

    /// The memory-object offset of one guest-physical byte, or `None` for the hole.
    #[must_use]
    pub fn host_offset(self, address: u64) -> Option<u64> {
        self.regions().iter().find_map(|region| {
            (address >= region.guest_start && address < region.guest_end())
                .then(|| region.host_offset + (address - region.guest_start))
        })
    }

    /// Whether `[address, address + length)` lies inside exactly one backed range.
    ///
    /// A range that straddles the MMIO hole is not contained: the bytes between the ranges do
    /// not exist, so a caller that wants them has asked about a machine this is not.
    #[must_use]
    pub fn contains(self, address: u64, length: u64) -> bool {
        address.checked_add(length).is_some_and(|end| end > address)
            && self
                .regions()
                .iter()
                .any(|region| region.contains(address, length))
    }

    /// The PVH memory map: RAM, the reserved legacy hole, high memory, and the MMIO hole.
    #[must_use]
    pub fn memory_map(self) -> Vec<MemoryMapEntry> {
        let low = self.regions()[0];
        let mut entries = vec![
            MemoryMapEntry {
                address: 0,
                size: LEGACY_HOLE_START,
                kind: MemoryKind::Ram,
            },
            MemoryMapEntry {
                address: LEGACY_HOLE_START,
                size: HIGH_MEMORY_START - LEGACY_HOLE_START,
                kind: MemoryKind::Reserved,
            },
            MemoryMapEntry {
                address: HIGH_MEMORY_START,
                size: low.size - HIGH_MEMORY_START,
                kind: MemoryKind::Ram,
            },
        ];
        if let Some(high) = self.regions().get(1) {
            entries.push(MemoryMapEntry {
                address: LOW_RAM_END,
                size: MMIO_HOLE_BYTES,
                kind: MemoryKind::Reserved,
            });
            entries.push(MemoryMapEntry {
                address: high.guest_start,
                size: high.size,
                kind: MemoryKind::Ram,
            });
        }
        entries
    }
}

// The hole must hold the fixed MMIO window and the TSS page, the low range must leave room for
// the legacy hole and the loader gap, and the two ranges must not meet, or a machine built from
// these constants would put a device page inside RAM.
const _: () = {
    assert!(LOW_RAM_END > HIGH_MEMORY_START);
    assert!(LOW_RAM_END.is_multiple_of(PAGE_SIZE));
    assert!(MMIO_HOLE_END.is_multiple_of(PAGE_SIZE));
    assert!(MAX_RAM_BYTES >= LOW_RAM_END);
    assert!(MAX_RAM_BYTES.is_multiple_of(PAGE_SIZE));
    assert!(MIN_RAM_BYTES >= HIGH_MEMORY_START);
    assert!(KERNEL_START > HIGH_MEMORY_START && KERNEL_START < LOW_RAM_END);
    // The fixed device window must be inside the hole, or a device page would sit inside RAM.
    assert!(
        crate::virtio::MMIO_WINDOW_BASE >= LOW_RAM_END
            && crate::virtio::MMIO_WINDOW_BASE + crate::virtio::MMIO_PAGE_SIZE * 5 <= MMIO_HOLE_END
    );
};

#[cfg(test)]
mod tests;
