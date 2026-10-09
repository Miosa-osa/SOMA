//! Fixed guest-physical layout from the `x86_64` machine contract.
//!
//! Every constant is a guest-physical byte address. The pure geometry - the RAM ranges, the
//! guest-to-object translation, and the PVH memory map - lives in [`crate::memory_layout`] so a
//! host that cannot boot the machine can still verify it; this module adds the boot-page and TSS
//! addresses the machine itself writes and re-exports the rest under the paths the machine uses.

pub(crate) use crate::memory_layout::{
    GuestLayout, HIGH_MEMORY_START, KERNEL_START, LEGACY_HOLE_START, LOW_RAM_END, PAGE_SIZE,
};
/// The RAM bound is named only by proofs, so it is re-exported only where they are built.
#[cfg(test)]
pub(crate) use crate::memory_layout::{MAX_RAM_BYTES, MIN_RAM_BYTES};

/// One 56-byte `hvm_start_info` followed by zeroes.
pub(crate) const START_INFO_ADDRESS: u64 = 0x6000;
/// Bounded `hvm_memmap_table_entry` values.
pub(crate) const MEMMAP_ADDRESS: u64 = 0x7000;
/// At most one initramfs module entry (unused by the halt guest).
pub(crate) const MODULE_ADDRESS: u64 = 0x8000;
/// NUL-terminated ASCII command line, at most 8,191 bytes.
pub(crate) const CMDLINE_ADDRESS: u64 = 0x9000;
pub(crate) const CMDLINE_MAX_BYTES: u64 = 8 * 1024;
/// Conventional three-page TSS window, placed inside the MMIO hole above low RAM.
pub(crate) const TSS_ADDRESS: u64 = 0xfffb_d000;
/// Bytes the TSS window occupies; it must fit inside the hole with the MMIO window.
pub(crate) const TSS_BYTES: u64 = 3 * PAGE_SIZE;

// The boot pages sit below the legacy hole, the kernel sits above it and below the MMIO
// boundary, and the TSS page sits inside the MMIO hole the memory layout reserves, so no
// device or control page can land inside guest RAM at any admitted size.
const _: () = {
    assert!(START_INFO_ADDRESS < MEMMAP_ADDRESS);
    assert!(MEMMAP_ADDRESS < MODULE_ADDRESS);
    assert!(MODULE_ADDRESS < CMDLINE_ADDRESS);
    assert!(CMDLINE_ADDRESS + CMDLINE_MAX_BYTES <= LEGACY_HOLE_START);
    assert!(HIGH_MEMORY_START < KERNEL_START);
    assert!(KERNEL_START < LOW_RAM_END);
    assert!(TSS_ADDRESS >= LOW_RAM_END);
    assert!(TSS_ADDRESS + TSS_BYTES <= crate::memory_layout::MMIO_HOLE_END);
};

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
    fn containment_uses_checked_arithmetic() {
        let layout = GuestLayout::new(MIN_RAM_BYTES).unwrap();
        assert!(layout.contains(KERNEL_START, 16));
        assert!(layout.contains(MIN_RAM_BYTES - 1, 1));
        assert!(!layout.contains(MIN_RAM_BYTES, 1));
        assert!(!layout.contains(u64::MAX, 1));
    }

    #[test]
    fn the_tss_window_never_lands_inside_guest_ram() {
        for size in [MIN_RAM_BYTES, LOW_RAM_END, MAX_RAM_BYTES] {
            let layout = GuestLayout::new(size).unwrap();
            assert!(!layout.contains(TSS_ADDRESS, TSS_BYTES));
            assert_eq!(layout.host_offset(TSS_ADDRESS), None);
        }
    }
}
