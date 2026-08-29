//! PVH `hvm_start_info`, memory-map, and command-line encoding.
//!
//! The halt guest ignores these structures, but writing them exercises the exact byte layout the
//! pinned PVH kernel will consume in the next slice.

use super::{
    error::{MachineError, Phase},
    layout::{
        CMDLINE_ADDRESS, CMDLINE_MAX_BYTES, GuestLayout, LEGACY_HOLE_START, MEMMAP_ADDRESS,
        MODULE_ADDRESS,
    },
};

pub(crate) const START_INFO_MAGIC: u32 = 0x336e_c578;
pub(crate) const START_INFO_VERSION: u32 = 1;
pub(crate) const START_INFO_BYTES: usize = 56;
pub(crate) const MEMMAP_ENTRY_BYTES: usize = 24;
pub(crate) const MODULE_ENTRY_BYTES: usize = 32;
const MEMMAP_TYPE_RAM: u32 = 1;
const MEMMAP_TYPE_RESERVED: u32 = 2;

/// The fixed diagnostic command line from the machine contract.
pub(crate) const DIAGNOSTIC_CMDLINE: &str = "console=ttyS0 reboot=k panic=1 nomodule random.trust_cpu=off pci=off acpi=off noapic cryptomgr.notests";

/// Encodes one `hvm_start_info` with an optional single module entry.
pub(crate) fn start_info(memmap_entries: u32, module_count: u32) -> [u8; START_INFO_BYTES] {
    let modlist_paddr = if module_count == 0 { 0 } else { MODULE_ADDRESS };
    let mut bytes = [0_u8; START_INFO_BYTES];
    bytes[0..4].copy_from_slice(&START_INFO_MAGIC.to_le_bytes());
    bytes[4..8].copy_from_slice(&START_INFO_VERSION.to_le_bytes());
    bytes[8..12].copy_from_slice(&0_u32.to_le_bytes());
    bytes[12..16].copy_from_slice(&module_count.to_le_bytes());
    bytes[16..24].copy_from_slice(&modlist_paddr.to_le_bytes());
    bytes[24..32].copy_from_slice(&CMDLINE_ADDRESS.to_le_bytes());
    bytes[32..40].copy_from_slice(&0_u64.to_le_bytes());
    bytes[40..48].copy_from_slice(&MEMMAP_ADDRESS.to_le_bytes());
    bytes[48..52].copy_from_slice(&memmap_entries.to_le_bytes());
    bytes[52..56].copy_from_slice(&0_u32.to_le_bytes());
    bytes
}

/// Encodes the contract's memory map: low RAM, the reserved legacy hole, then high RAM.
pub(crate) fn memmap(layout: GuestLayout) -> Result<Vec<u8>, MachineError> {
    let [(low_start, low_size), (high_start, high_size)] = layout.ram_ranges()?;
    let hole_size = high_start
        .checked_sub(LEGACY_HOLE_START)
        .ok_or_else(|| MachineError::invalid(Phase::LoadGuest, "legacy hole overflow"))?;
    let entries = [
        (low_start, low_size, MEMMAP_TYPE_RAM),
        (LEGACY_HOLE_START, hole_size, MEMMAP_TYPE_RESERVED),
        (high_start, high_size, MEMMAP_TYPE_RAM),
    ];
    let mut bytes = Vec::with_capacity(entries.len() * MEMMAP_ENTRY_BYTES);
    for (address, size, kind) in entries {
        bytes.extend_from_slice(&address.to_le_bytes());
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
    }
    Ok(bytes)
}

/// Encodes the sole `hvm_modlist_entry` for an initramfs at `address` with `size` bytes.
pub(crate) fn module_entry(address: u64, size: u64) -> [u8; MODULE_ENTRY_BYTES] {
    let mut bytes = [0_u8; MODULE_ENTRY_BYTES];
    bytes[0..8].copy_from_slice(&address.to_le_bytes());
    bytes[8..16].copy_from_slice(&size.to_le_bytes());
    bytes
}

/// Encodes a NUL-terminated ASCII command line within the contract bound.
pub(crate) fn cmdline(text: &str) -> Result<Vec<u8>, MachineError> {
    if !text.is_ascii()
        || text
            .bytes()
            .any(|byte| byte == 0 || byte.is_ascii_control())
    {
        return Err(MachineError::invalid(
            Phase::LoadGuest,
            "kernel command line must be printable ASCII",
        ));
    }
    let limit = usize::try_from(CMDLINE_MAX_BYTES)
        .map_err(|_| MachineError::invalid(Phase::LoadGuest, "command line bound overflow"))?;
    // The contract bounds the terminated line, so the text plus its NUL must stay under 8 KiB.
    if text.len().saturating_add(1) >= limit {
        return Err(MachineError::invalid(
            Phase::LoadGuest,
            "kernel command line exceeds 8,191 bytes including its terminator",
        ));
    }
    let mut bytes = Vec::with_capacity(text.len() + 1);
    bytes.extend_from_slice(text.as_bytes());
    bytes.push(0);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::x86_64::layout::{HIGH_MEMORY_START, MIN_RAM_BYTES};

    #[test]
    fn start_info_matches_the_contract_fields() {
        let bytes = start_info(2, 0);
        assert_eq!(&bytes[0..4], &START_INFO_MAGIC.to_le_bytes());
        assert_eq!(&bytes[4..8], &1_u32.to_le_bytes());
        assert_eq!(&bytes[12..16], &0_u32.to_le_bytes());
        assert_eq!(&bytes[16..24], &0_u64.to_le_bytes());
        assert_eq!(&bytes[24..32], &CMDLINE_ADDRESS.to_le_bytes());
        assert_eq!(&bytes[32..40], &0_u64.to_le_bytes());
        assert_eq!(&bytes[40..48], &MEMMAP_ADDRESS.to_le_bytes());
        assert_eq!(&bytes[48..52], &2_u32.to_le_bytes());
        assert_eq!(&bytes[52..56], &0_u32.to_le_bytes());
        assert_eq!(&start_info(2, 1)[16..24], &MODULE_ADDRESS.to_le_bytes());
    }

    #[test]
    fn memmap_encodes_ram_hole_and_ram() {
        let bytes = memmap(GuestLayout::new(MIN_RAM_BYTES).unwrap()).unwrap();
        assert_eq!(bytes.len(), 3 * MEMMAP_ENTRY_BYTES);
        assert_eq!(&bytes[0..8], &0_u64.to_le_bytes());
        assert_eq!(&bytes[8..16], &LEGACY_HOLE_START.to_le_bytes());
        assert_eq!(&bytes[16..20], &MEMMAP_TYPE_RAM.to_le_bytes());
        assert_eq!(&bytes[24..32], &LEGACY_HOLE_START.to_le_bytes());
        assert_eq!(&bytes[32..40], &0x6_0000_u64.to_le_bytes());
        assert_eq!(&bytes[40..44], &MEMMAP_TYPE_RESERVED.to_le_bytes());
        assert_eq!(&bytes[48..56], &HIGH_MEMORY_START.to_le_bytes());
        assert_eq!(
            &bytes[56..64],
            &(MIN_RAM_BYTES - HIGH_MEMORY_START).to_le_bytes()
        );
        assert_eq!(&bytes[64..68], &MEMMAP_TYPE_RAM.to_le_bytes());
    }

    #[test]
    fn module_entry_carries_address_and_size_only() {
        let bytes = module_entry(0x0700_0000, 4096);
        assert_eq!(&bytes[0..8], &0x0700_0000_u64.to_le_bytes());
        assert_eq!(&bytes[8..16], &4096_u64.to_le_bytes());
        assert!(bytes[16..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn cmdline_is_bounded_ascii_with_terminator() {
        let bytes = cmdline(DIAGNOSTIC_CMDLINE).unwrap();
        assert_eq!(bytes.last(), Some(&0));
        assert_eq!(&bytes[..bytes.len() - 1], DIAGNOSTIC_CMDLINE.as_bytes());
        assert!(cmdline("bad\u{e9}").is_err());
        assert!(cmdline("bad\n").is_err());
        assert!(cmdline(&"a".repeat(8191)).is_err());
        assert!(cmdline(&"a".repeat(8190)).is_ok());
    }
}
