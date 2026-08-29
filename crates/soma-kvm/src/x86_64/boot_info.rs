//! PVH `hvm_start_info`, memory-map, and command-line encoding.
//!
//! The halt guest ignores these structures, but writing them exercises the exact byte layout the
//! pinned PVH kernel will consume in the next slice.

use super::{
    error::{HaltGuestError, Phase},
    layout::{CMDLINE_ADDRESS, CMDLINE_MAX_BYTES, GuestLayout, MEMMAP_ADDRESS, MODULE_ADDRESS},
};

pub(crate) const START_INFO_MAGIC: u32 = 0x336e_c578;
pub(crate) const START_INFO_VERSION: u32 = 1;
pub(crate) const START_INFO_BYTES: usize = 56;
pub(crate) const MEMMAP_ENTRY_BYTES: usize = 24;
const MEMMAP_TYPE_RAM: u32 = 1;

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

/// Encodes the contract's RAM entries as `hvm_memmap_table_entry` values.
pub(crate) fn memmap(layout: GuestLayout) -> Result<Vec<u8>, HaltGuestError> {
    let mut bytes = Vec::with_capacity(2 * MEMMAP_ENTRY_BYTES);
    for (address, size) in layout.ram_ranges()? {
        bytes.extend_from_slice(&address.to_le_bytes());
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&MEMMAP_TYPE_RAM.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
    }
    Ok(bytes)
}

/// Encodes a NUL-terminated ASCII command line within the contract bound.
pub(crate) fn cmdline(text: &str) -> Result<Vec<u8>, HaltGuestError> {
    if !text.is_ascii()
        || text
            .bytes()
            .any(|byte| byte == 0 || byte.is_ascii_control())
    {
        return Err(HaltGuestError::invalid(
            Phase::LoadGuest,
            "kernel command line must be printable ASCII",
        ));
    }
    let limit = usize::try_from(CMDLINE_MAX_BYTES)
        .map_err(|_| HaltGuestError::invalid(Phase::LoadGuest, "command line bound overflow"))?;
    // The contract bounds the terminated line, so the text plus its NUL must stay under 8 KiB.
    if text.len().saturating_add(1) >= limit {
        return Err(HaltGuestError::invalid(
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
    use crate::x86_64::layout::{HIGH_MEMORY_START, LEGACY_HOLE_START, MIN_RAM_BYTES};

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
    fn memmap_encodes_two_ram_entries() {
        let bytes = memmap(GuestLayout::new(MIN_RAM_BYTES).unwrap()).unwrap();
        assert_eq!(bytes.len(), 2 * MEMMAP_ENTRY_BYTES);
        assert_eq!(&bytes[0..8], &0_u64.to_le_bytes());
        assert_eq!(&bytes[8..16], &LEGACY_HOLE_START.to_le_bytes());
        assert_eq!(&bytes[16..20], &MEMMAP_TYPE_RAM.to_le_bytes());
        assert_eq!(&bytes[24..32], &HIGH_MEMORY_START.to_le_bytes());
        assert_eq!(
            &bytes[32..40],
            &(MIN_RAM_BYTES - HIGH_MEMORY_START).to_le_bytes()
        );
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
