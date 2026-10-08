//! The Intel MP Specification 1.4 configuration table for the `x86_64` guest.
//!
//! A version 2 SOMA machine boots with ACPI off, so the MP table is the only structure the
//! pinned Linux kernel can use to discover its application processors and the I/O APIC that
//! carries every device interrupt. The encoder below is pure bytes with no KVM and no
//! architecture gate, exactly like [`crate::cmdline`]: a client on a host that cannot boot
//! the machine must still be able to compose and verify the same table the machine writes.
//!
//! The floating pointer structure sits at [`MP_TABLE_ADDRESS`], inside the reserved legacy
//! hole the machine contract already reserves and which the machine reports as reserved in
//! its PVH memory map. That placement matters twice: the kernel scans `0xf0000..0x100000`
//! for the floating pointer, and a region the platform describes as reserved is never handed
//! to a guest allocator that could overwrite the table before the kernel reads it. The low
//! workspace would also be scanned for its final kilobyte, but the same region is reported as
//! usable RAM, so a table placed there is only as safe as the kernel's boot reservation.

use std::fmt;

/// First byte of the fixed table region, paragraph aligned.
///
/// It lies inside the reserved legacy hole `0xa0000..0xfffff`, which the machine contract
/// already reserves and which the kernel reports as reserved in the PVH memory map.
pub const MP_TABLE_ADDRESS: u64 = 0x000f_0000;
/// Upper bound on the encoded table, so the region never reaches the loader gap.
pub const MP_TABLE_MAX_BYTES: usize = 1024;
/// The largest processor count a version 2 machine admits.
pub const MAX_PROCESSORS: u16 = 8;

/// Bytes of the floating pointer structure.
pub const FLOATING_POINTER_BYTES: usize = 16;
/// Bytes of the configuration-table header.
pub const CONFIG_HEADER_BYTES: usize = 44;
/// Bytes of one processor entry.
pub const PROCESSOR_ENTRY_BYTES: usize = 20;
/// Bytes of one bus entry.
pub const BUS_ENTRY_BYTES: usize = 8;
/// Bytes of one I/O APIC entry.
pub const IOAPIC_ENTRY_BYTES: usize = 8;
/// Bytes of one I/O interrupt assignment entry.
pub const IO_INTERRUPT_ENTRY_BYTES: usize = 8;

/// Offset of the configuration table inside the published region.
pub const CONFIG_OFFSET: usize = FLOATING_POINTER_BYTES;

const SIGNATURE_FLOATING_POINTER: &[u8; 4] = b"_MP_";
const SIGNATURE_CONFIG: &[u8; 4] = b"PCMP";
const SPEC_REVISION: u8 = 4;
/// One 16-byte paragraph, counted in the floating pointer's length byte.
const FLOATING_POINTER_PARAGRAPHS: u8 = 1;

const CONFIG_LENGTH_OFFSET: usize = 4;
const CONFIG_CHECKSUM_OFFSET: usize = 7;
const CONFIG_OEM_ID_OFFSET: usize = 8;
const CONFIG_PRODUCT_ID_OFFSET: usize = 16;
const CONFIG_ENTRY_COUNT_OFFSET: usize = 34;
const CONFIG_LAPIC_OFFSET: usize = 36;
const CONFIG_EXTENDED_LENGTH_OFFSET: usize = 40;

const OEM_ID: &[u8; 8] = b"SOMA    ";
const PRODUCT_ID: &[u8; 12] = b"SOMA-MPTBL  ";

/// Physical address of the local APIC, identical on every `x86_64` machine.
pub const LAPIC_ADDRESS: u32 = 0xfee0_0000;
/// Physical address of the I/O APIC the machine builds.
pub const IOAPIC_ADDRESS: u32 = 0xfec0_0000;
/// The single bus a version 2 machine declares.
pub const ISA_BUS_ID: u8 = 0;
/// Identifier of the single I/O APIC entry.
pub const IOAPIC_ID: u8 = 0;
/// ISA interrupt lines the table maps, so the timer, the console, and the five device GSIs
/// all have an explicit route through the I/O APIC.
pub const ISA_INTERRUPT_COUNT: u8 = 16;

const ENTRY_PROCESSOR: u8 = 0;
const ENTRY_BUS: u8 = 1;
const ENTRY_IOAPIC: u8 = 2;
const ENTRY_IO_INTERRUPT: u8 = 3;
const CPU_FLAG_ENABLED: u8 = 0b01;
const CPU_FLAG_BOOTSTRAP: u8 = 0b10;
/// Local APIC version the machine's in-kernel controller reports.
const LAPIC_VERSION: u8 = 0x14;
/// I/O APIC version the machine's in-kernel controller reports.
const IOAPIC_VERSION: u8 = 0x11;
const IOAPIC_FLAG_ENABLED: u8 = 0b01;
/// Interrupt type 0: an INT entry conforming to the bus, which for ISA is edge, active high.
const INTERRUPT_TYPE_CONFORMING: u8 = 0;
/// CPUID signature the processor entries advertise, matching the pinned kernel's family.
const CPU_STEPPING: u32 = 0x600;
/// CPUID feature word the processor entries advertise: FPU and on-chip APIC.
const CPU_FEATURE_FLAGS: u32 = 0x0201;

// The published region is paragraph aligned, inside the kernel's scan window, and large
// enough for the largest table, so a wrong constant is a compile error rather than a table
// the kernel never finds.
const _: () = {
    assert!(MP_TABLE_ADDRESS.is_multiple_of(16));
    assert!(MP_TABLE_ADDRESS >= 0xf_0000);
    assert!(MP_TABLE_ADDRESS + MP_TABLE_MAX_BYTES as u64 <= 0x10_0000);
};

impl fmt::Display for MpTableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooFewProcessors => {
                formatter.write_str("an MP table needs at least one processor")
            }
            Self::TooManyProcessors { count } => write!(
                formatter,
                "an MP table admits at most {MAX_PROCESSORS} processors, got {count}"
            ),
            Self::TableTooLarge { bytes } => write!(
                formatter,
                "the encoded MP table is {bytes} bytes, above the {MP_TABLE_MAX_BYTES}-byte bound"
            ),
        }
    }
}

/// Why a processor count cannot be encoded as an MP table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MpTableError {
    /// No processor was requested.
    TooFewProcessors,
    /// More processors than the contract admits.
    TooManyProcessors { count: u16 },
    /// The encoded table exceeds [`MP_TABLE_MAX_BYTES`].
    TableTooLarge { bytes: usize },
}

impl std::error::Error for MpTableError {}

/// Encodes the floating pointer and configuration table for `vcpus` processors.
///
/// The processors are listed with local APIC identifiers `0..vcpus`, which is the identifier
/// KVM assigns each vCPU in creation order, and only processor zero carries the bootstrap
/// flag. Every ISA interrupt line is mapped to the matching I/O APIC input, so the in-kernel
/// timer on line 0, the console on line 4, and the five device GSIs on lines 5 through 9 all
/// have an explicit route once the guest stops servicing interrupts through the PIC.
///
/// # Errors
///
/// Returns [`MpTableError::TooFewProcessors`] for zero, [`MpTableError::TooManyProcessors`]
/// above [`MAX_PROCESSORS`], or [`MpTableError::TableTooLarge`] when the encoding would
/// leave the region.
pub fn encode(vcpus: u16) -> Result<Vec<u8>, MpTableError> {
    if vcpus == 0 {
        return Err(MpTableError::TooFewProcessors);
    }
    if vcpus > MAX_PROCESSORS {
        return Err(MpTableError::TooManyProcessors { count: vcpus });
    }
    let processors = usize::from(vcpus);
    let entry_count = processors + 2 + usize::from(ISA_INTERRUPT_COUNT);
    let base_length = CONFIG_HEADER_BYTES
        + processors * PROCESSOR_ENTRY_BYTES
        + BUS_ENTRY_BYTES
        + IOAPIC_ENTRY_BYTES
        + usize::from(ISA_INTERRUPT_COUNT) * IO_INTERRUPT_ENTRY_BYTES;
    let total = CONFIG_OFFSET + base_length;
    if total > MP_TABLE_MAX_BYTES {
        return Err(MpTableError::TableTooLarge { bytes: total });
    }

    let mut table = vec![0_u8; base_length];
    table[0..4].copy_from_slice(SIGNATURE_CONFIG);
    table[CONFIG_LENGTH_OFFSET..CONFIG_LENGTH_OFFSET + 2]
        .copy_from_slice(&u16::try_from(base_length).unwrap_or(u16::MAX).to_le_bytes());
    table[6] = SPEC_REVISION;
    table[CONFIG_OEM_ID_OFFSET..CONFIG_OEM_ID_OFFSET + 8].copy_from_slice(OEM_ID);
    table[CONFIG_PRODUCT_ID_OFFSET..CONFIG_PRODUCT_ID_OFFSET + 12].copy_from_slice(PRODUCT_ID);
    table[CONFIG_ENTRY_COUNT_OFFSET..CONFIG_ENTRY_COUNT_OFFSET + 2]
        .copy_from_slice(&u16::try_from(entry_count).unwrap_or(u16::MAX).to_le_bytes());
    table[CONFIG_LAPIC_OFFSET..CONFIG_LAPIC_OFFSET + 4]
        .copy_from_slice(&LAPIC_ADDRESS.to_le_bytes());
    table[CONFIG_EXTENDED_LENGTH_OFFSET..CONFIG_EXTENDED_LENGTH_OFFSET + 2]
        .copy_from_slice(&0_u16.to_le_bytes());

    let mut cursor = CONFIG_HEADER_BYTES;
    for apic_id in 0..processors {
        let entry = processor_entry(u8::try_from(apic_id).unwrap_or(u8::MAX), apic_id == 0);
        table[cursor..cursor + PROCESSOR_ENTRY_BYTES].copy_from_slice(&entry);
        cursor += PROCESSOR_ENTRY_BYTES;
    }
    table[cursor..cursor + BUS_ENTRY_BYTES].copy_from_slice(&bus_entry());
    cursor += BUS_ENTRY_BYTES;
    table[cursor..cursor + IOAPIC_ENTRY_BYTES].copy_from_slice(&ioapic_entry());
    cursor += IOAPIC_ENTRY_BYTES;
    for irq in 0..ISA_INTERRUPT_COUNT {
        let entry = io_interrupt_entry(irq, irq);
        table[cursor..cursor + IO_INTERRUPT_ENTRY_BYTES].copy_from_slice(&entry);
        cursor += IO_INTERRUPT_ENTRY_BYTES;
    }
    // The configuration checksum covers the base table only, which is the whole table here
    // because a version 2 machine publishes no extended entries.
    table[CONFIG_CHECKSUM_OFFSET] = checksum(&table);

    let mut blob = vec![0_u8; CONFIG_OFFSET];
    blob[0..4].copy_from_slice(SIGNATURE_FLOATING_POINTER);
    let config_address = u32::try_from(MP_TABLE_ADDRESS)
        .unwrap_or(0)
        .wrapping_add(u32::try_from(CONFIG_OFFSET).unwrap_or(0));
    blob[4..8].copy_from_slice(&config_address.to_le_bytes());
    blob[8] = FLOATING_POINTER_PARAGRAPHS;
    blob[9] = SPEC_REVISION;
    blob[10] = checksum(&blob);
    blob.extend_from_slice(&table);
    Ok(blob)
}

/// The two's-complement byte that makes the whole slice sum to zero.
fn checksum(bytes: &[u8]) -> u8 {
    let sum = bytes
        .iter()
        .fold(0_u8, |total, byte| total.wrapping_add(*byte));
    sum.wrapping_neg()
}

fn processor_entry(apic_id: u8, bootstrap: bool) -> [u8; PROCESSOR_ENTRY_BYTES] {
    let mut entry = [0_u8; PROCESSOR_ENTRY_BYTES];
    entry[0] = ENTRY_PROCESSOR;
    entry[1] = apic_id;
    entry[2] = LAPIC_VERSION;
    entry[3] = if bootstrap {
        CPU_FLAG_ENABLED | CPU_FLAG_BOOTSTRAP
    } else {
        CPU_FLAG_ENABLED
    };
    entry[4..8].copy_from_slice(&CPU_STEPPING.to_le_bytes());
    entry[8..12].copy_from_slice(&CPU_FEATURE_FLAGS.to_le_bytes());
    entry
}

fn bus_entry() -> [u8; BUS_ENTRY_BYTES] {
    let mut entry = [0_u8; BUS_ENTRY_BYTES];
    entry[0] = ENTRY_BUS;
    entry[1] = ISA_BUS_ID;
    entry[2..8].copy_from_slice(b"ISA   ");
    entry
}

fn ioapic_entry() -> [u8; IOAPIC_ENTRY_BYTES] {
    let mut entry = [0_u8; IOAPIC_ENTRY_BYTES];
    entry[0] = ENTRY_IOAPIC;
    entry[1] = IOAPIC_ID;
    entry[2] = IOAPIC_VERSION;
    entry[3] = IOAPIC_FLAG_ENABLED;
    entry[4..8].copy_from_slice(&IOAPIC_ADDRESS.to_le_bytes());
    entry
}

fn io_interrupt_entry(bus_irq: u8, ioapic_input: u8) -> [u8; IO_INTERRUPT_ENTRY_BYTES] {
    let mut entry = [0_u8; IO_INTERRUPT_ENTRY_BYTES];
    entry[0] = ENTRY_IO_INTERRUPT;
    entry[1] = INTERRUPT_TYPE_CONFORMING;
    // Flags zero means the polarity and trigger follow the bus, which for ISA is edge,
    // active high; it keeps the table a function of the bus rather than of a guess.
    entry[4] = ISA_BUS_ID;
    entry[5] = bus_irq;
    entry[6] = IOAPIC_ID;
    entry[7] = ioapic_input;
    entry
}

#[cfg(test)]
mod tests;
