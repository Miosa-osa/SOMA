use super::*;

fn sum(bytes: &[u8]) -> u8 {
    bytes
        .iter()
        .fold(0_u8, |total, byte| total.wrapping_add(*byte))
}

#[test]
fn checksum_makes_a_slice_sum_to_zero() {
    let mut bytes = [1_u8, 2, 3, 0];
    bytes[3] = checksum(&bytes);
    assert_eq!(sum(&bytes), 0);
    assert_eq!(checksum(&[0_u8; 8]), 0);
}

#[test]
fn rejects_zero_and_more_than_the_contract_admits() {
    assert_eq!(encode(0), Err(MpTableError::TooFewProcessors));
    assert_eq!(
        encode(MAX_PROCESSORS + 1),
        Err(MpTableError::TooManyProcessors {
            count: MAX_PROCESSORS + 1
        })
    );
    assert!(encode(MAX_PROCESSORS).is_ok());
    assert!(encode(1).is_ok());
}

#[test]
fn floating_pointer_names_the_configuration_table_and_checksums() {
    let bytes = encode(1).unwrap();
    assert_eq!(&bytes[0..4], SIGNATURE_FLOATING_POINTER);
    assert_eq!(sum(&bytes[..FLOATING_POINTER_BYTES]), 0);
    assert_eq!(bytes[8], FLOATING_POINTER_PARAGRAPHS);
    assert_eq!(bytes[9], SPEC_REVISION);
    let pointer = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    assert_eq!(u64::from(pointer), MP_TABLE_ADDRESS + CONFIG_OFFSET as u64);
}

#[test]
fn configuration_header_bounds_and_checksums_the_base_table() {
    for vcpus in 1..=MAX_PROCESSORS {
        let bytes = encode(vcpus).unwrap();
        let table = &bytes[CONFIG_OFFSET..];
        assert_eq!(&table[0..4], SIGNATURE_CONFIG);
        assert_eq!(table[6], SPEC_REVISION);
        assert_eq!(sum(table), 0, "base table checksum for {vcpus} processors");
        let length = usize::from(u16::from_le_bytes(
            table[CONFIG_LENGTH_OFFSET..CONFIG_LENGTH_OFFSET + 2]
                .try_into()
                .unwrap(),
        ));
        assert_eq!(length, table.len());
        let entries = usize::from(u16::from_le_bytes(
            table[CONFIG_ENTRY_COUNT_OFFSET..CONFIG_ENTRY_COUNT_OFFSET + 2]
                .try_into()
                .unwrap(),
        ));
        assert_eq!(
            entries,
            usize::from(vcpus) + 2 + usize::from(ISA_INTERRUPT_COUNT)
        );
        let lapic = u32::from_le_bytes(
            table[CONFIG_LAPIC_OFFSET..CONFIG_LAPIC_OFFSET + 4]
                .try_into()
                .unwrap(),
        );
        assert_eq!(lapic, LAPIC_ADDRESS);
        assert_eq!(
            u16::from_le_bytes(
                table[CONFIG_EXTENDED_LENGTH_OFFSET..CONFIG_EXTENDED_LENGTH_OFFSET + 2]
                    .try_into()
                    .unwrap()
            ),
            0
        );
        assert!(bytes.len() <= MP_TABLE_MAX_BYTES);
    }
}

#[test]
fn processor_entries_are_apic_ids_zero_up_with_one_bootstrap() {
    let vcpus = 8_u16;
    let bytes = encode(vcpus).unwrap();
    let table = &bytes[CONFIG_OFFSET..];
    let mut cursor = CONFIG_HEADER_BYTES;
    let mut bootstraps = 0;
    for expected in 0..vcpus {
        let entry = &table[cursor..cursor + PROCESSOR_ENTRY_BYTES];
        assert_eq!(entry[0], ENTRY_PROCESSOR);
        assert_eq!(entry[1], u8::try_from(expected).unwrap());
        assert_eq!(entry[2], LAPIC_VERSION);
        assert_eq!(entry[3] & CPU_FLAG_ENABLED, CPU_FLAG_ENABLED);
        let is_bootstrap = entry[3] & CPU_FLAG_BOOTSTRAP != 0;
        assert_eq!(is_bootstrap, expected == 0);
        bootstraps += u32::from(is_bootstrap);
        cursor += PROCESSOR_ENTRY_BYTES;
    }
    assert_eq!(bootstraps, 1);
}

#[test]
fn one_bus_one_ioapic_and_every_isa_line_are_mapped() {
    let bytes = encode(2).unwrap();
    let table = &bytes[CONFIG_OFFSET..];
    let bus = &table[CONFIG_HEADER_BYTES + 2 * PROCESSOR_ENTRY_BYTES..];
    assert_eq!(bus[0], ENTRY_BUS);
    assert_eq!(bus[1], ISA_BUS_ID);
    assert_eq!(&bus[2..8], b"ISA   ");
    let ioapic = &bus[BUS_ENTRY_BYTES..];
    assert_eq!(ioapic[0], ENTRY_IOAPIC);
    assert_eq!(ioapic[3], IOAPIC_FLAG_ENABLED);
    assert_eq!(
        u32::from_le_bytes(ioapic[4..8].try_into().unwrap()),
        IOAPIC_ADDRESS
    );
    let interrupts = &ioapic[IOAPIC_ENTRY_BYTES..];
    assert_eq!(
        interrupts.len(),
        usize::from(ISA_INTERRUPT_COUNT) * IO_INTERRUPT_ENTRY_BYTES
    );
    for irq in 0..ISA_INTERRUPT_COUNT {
        let offset = usize::from(irq) * IO_INTERRUPT_ENTRY_BYTES;
        let entry = &interrupts[offset..offset + IO_INTERRUPT_ENTRY_BYTES];
        assert_eq!(entry[0], ENTRY_IO_INTERRUPT);
        assert_eq!(entry[1], INTERRUPT_TYPE_CONFORMING);
        assert_eq!(entry[4], ISA_BUS_ID);
        assert_eq!(entry[5], irq);
        assert_eq!(entry[6], IOAPIC_ID);
        assert_eq!(entry[7], irq);
    }
}

#[test]
fn one_processor_encoding_is_deterministic_and_small() {
    let first = encode(1).unwrap();
    let second = encode(1).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first.len(),
        CONFIG_OFFSET
            + CONFIG_HEADER_BYTES
            + PROCESSOR_ENTRY_BYTES
            + BUS_ENTRY_BYTES
            + IOAPIC_ENTRY_BYTES
            + usize::from(ISA_INTERRUPT_COUNT) * IO_INTERRUPT_ENTRY_BYTES
    );
}
