//! Fixed guest-physical layout from the `x86_64` machine contract v1.
//!
//! Every constant is a guest-physical byte address. Callers validate overflow, overlap, and
//! containment through [`GuestLayout`] before any byte is published to guest RAM.

use super::error::{HaltGuestError, Phase};

pub(crate) const PAGE_SIZE: u64 = 4096;
pub(crate) const MIN_RAM_BYTES: u64 = 128 * 1024 * 1024;
pub(crate) const MAX_RAM_BYTES: u64 = 3 * 1024 * 1024 * 1024;

/// One 56-byte `hvm_start_info` followed by zeroes.
pub(crate) const START_INFO_ADDRESS: u64 = 0x6000;
/// Bounded `hvm_memmap_table_entry` values.
pub(crate) const MEMMAP_ADDRESS: u64 = 0x7000;
/// At most one initramfs module entry in version 1 (unused by the halt guest).
pub(crate) const MODULE_ADDRESS: u64 = 0x8000;
/// NUL-terminated ASCII command line, at most 8,191 bytes.
pub(crate) const CMDLINE_ADDRESS: u64 = 0x9000;
pub(crate) const CMDLINE_MAX_BYTES: u64 = 8 * 1024;
/// Start of the reserved legacy hole that the memory map reports as reserved.
pub(crate) const LEGACY_HOLE_START: u64 = 0x000a_0000;
/// First byte above the legacy hole and the loader gap begins here.
pub(crate) const HIGH_MEMORY_START: u64 = 0x0010_0000;
/// `CONFIG_PHYSICAL_START` of the pinned kernel; the halt guest is loaded here too.
pub(crate) const KERNEL_START: u64 = 0x0100_0000;
/// Conventional three-page TSS window placed above any supported guest RAM size.
pub(crate) const TSS_ADDRESS: u64 = 0xfffb_d000;

// Boot pages sit below the legacy hole, the kernel sits above it, and RAM never reaches the TSS.
const _: () = {
    assert!(START_INFO_ADDRESS < MEMMAP_ADDRESS);
    assert!(MEMMAP_ADDRESS < MODULE_ADDRESS);
    assert!(MODULE_ADDRESS < CMDLINE_ADDRESS);
    assert!(CMDLINE_ADDRESS + CMDLINE_MAX_BYTES <= LEGACY_HOLE_START);
    assert!(HIGH_MEMORY_START < KERNEL_START);
    assert!(MAX_RAM_BYTES <= TSS_ADDRESS);
};

/// A validated guest RAM size and the derived boundaries the proof writes into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GuestLayout {
    ram_bytes: u64,
}

impl GuestLayout {
    pub(crate) fn new(ram_bytes: u64) -> Result<Self, HaltGuestError> {
        if !ram_bytes.is_multiple_of(PAGE_SIZE) {
            return Err(HaltGuestError::invalid(
                Phase::MapMemory,
                "guest RAM size must be a multiple of 4 KiB",
            ));
        }
        if !(MIN_RAM_BYTES..=MAX_RAM_BYTES).contains(&ram_bytes) {
            return Err(HaltGuestError::invalid(
                Phase::MapMemory,
                "guest RAM size must be between 128 MiB and 3 GiB",
            ));
        }
        if ram_bytes > TSS_ADDRESS {
            return Err(HaltGuestError::invalid(
                Phase::MapMemory,
                "guest RAM overlaps the TSS window",
            ));
        }
        Ok(Self { ram_bytes })
    }

    pub(crate) const fn ram_bytes(self) -> u64 {
        self.ram_bytes
    }

    /// Returns the two RAM entries the contract reports when RAM crosses the legacy hole.
    pub(crate) fn ram_ranges(self) -> Result<[(u64, u64); 2], HaltGuestError> {
        let high = self
            .ram_bytes
            .checked_sub(HIGH_MEMORY_START)
            .ok_or_else(|| {
                HaltGuestError::invalid(Phase::LoadGuest, "guest RAM ends below high memory")
            })?;
        Ok([(0, LEGACY_HOLE_START), (HIGH_MEMORY_START, high)])
    }

    /// Checks that `[address, address + length)` lies inside guest RAM.
    pub(crate) fn contains(self, address: u64, length: u64) -> bool {
        address
            .checked_add(length)
            .is_some_and(|end| end <= self.ram_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_contract_range_and_rejects_the_rest() {
        assert!(GuestLayout::new(MIN_RAM_BYTES).is_ok());
        assert!(GuestLayout::new(MAX_RAM_BYTES).is_ok());
        assert!(GuestLayout::new(MIN_RAM_BYTES - PAGE_SIZE).is_err());
        assert!(GuestLayout::new(MAX_RAM_BYTES + PAGE_SIZE).is_err());
        assert!(GuestLayout::new(MIN_RAM_BYTES + 1).is_err());
        assert!(GuestLayout::new(0).is_err());
        assert!(GuestLayout::new(u64::MAX).is_err());
    }

    #[test]
    fn reports_two_ram_ranges_around_the_legacy_hole() {
        let layout = GuestLayout::new(MIN_RAM_BYTES).unwrap();
        assert_eq!(
            layout.ram_ranges().unwrap(),
            [
                (0, LEGACY_HOLE_START),
                (HIGH_MEMORY_START, MIN_RAM_BYTES - HIGH_MEMORY_START)
            ]
        );
    }

    #[test]
    fn containment_uses_checked_arithmetic() {
        let layout = GuestLayout::new(MIN_RAM_BYTES).unwrap();
        assert!(layout.contains(KERNEL_START, 16));
        assert!(layout.contains(MIN_RAM_BYTES - 1, 1));
        assert!(!layout.contains(MIN_RAM_BYTES, 1));
        assert!(!layout.contains(u64::MAX, 1));
    }
}
