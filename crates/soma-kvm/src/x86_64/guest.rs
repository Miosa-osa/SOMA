//! The raw 32-bit halt guest and the guest-RAM population step.

use super::{
    boot_info::{self, DIAGNOSTIC_CMDLINE},
    error::MachineError,
    layout::{CMDLINE_ADDRESS, KERNEL_START, MEMMAP_ADDRESS, START_INFO_ADDRESS},
    memory::GuestRam,
};

/// The bytes the halt guest emits before executing `hlt`.
pub const EXPECTED_SERIAL: &[u8] = b"SOMA";

/// 32-bit protected-mode machine code: `mov edx,0x3f8`, then for each byte `mov al,imm8` and
/// `out dx,al`, then `hlt`.
pub(crate) const HALT_PROGRAM: [u8; 18] = [
    0xba, 0xf8, 0x03, 0x00, 0x00, // mov edx, 0x3f8
    0xb0, 0x53, 0xee, // mov al, 'S'; out dx, al
    0xb0, 0x4f, 0xee, // mov al, 'O'; out dx, al
    0xb0, 0x4d, 0xee, // mov al, 'M'; out dx, al
    0xb0, 0x41, 0xee, // mov al, 'A'; out dx, al
    0xf4, // hlt
];

/// Writes the boot structures and the halt program into guest RAM and returns the entry point.
pub(crate) fn load(ram: &mut GuestRam) -> Result<u64, MachineError> {
    let memmap = boot_info::memmap(ram.layout())?;
    let entries = u32::try_from(memmap.len() / boot_info::MEMMAP_ENTRY_BYTES)
        .map_err(|_| MachineError::invalid(super::error::Phase::LoadGuest, "memmap overflow"))?;
    ram.write(START_INFO_ADDRESS, &boot_info::start_info(entries, 0))?;
    ram.write(MEMMAP_ADDRESS, &memmap)?;
    ram.write(CMDLINE_ADDRESS, &boot_info::cmdline(DIAGNOSTIC_CMDLINE)?)?;
    ram.write(KERNEL_START, &HALT_PROGRAM)?;
    Ok(KERNEL_START)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_emits_expected_bytes_then_halts() {
        let immediates: Vec<u8> = HALT_PROGRAM
            .windows(3)
            .filter(|window| window[0] == 0xb0 && window[2] == 0xee)
            .map(|window| window[1])
            .collect();
        assert_eq!(immediates, EXPECTED_SERIAL);
        assert_eq!(HALT_PROGRAM.last(), Some(&0xf4));
        assert_eq!(&HALT_PROGRAM[..5], &[0xba, 0xf8, 0x03, 0x00, 0x00]);
        assert_eq!(
            u16::from_le_bytes([HALT_PROGRAM[1], HALT_PROGRAM[2]]),
            crate::x86_64::serial::SERIAL_BASE
        );
    }

    #[test]
    fn loads_at_the_contract_kernel_start() {
        let layout =
            crate::x86_64::layout::GuestLayout::new(crate::x86_64::layout::MIN_RAM_BYTES).unwrap();
        let mut ram = GuestRam::map(layout).unwrap();
        assert_eq!(load(&mut ram).unwrap(), KERNEL_START);
    }
}
