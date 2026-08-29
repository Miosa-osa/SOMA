use super::*;
use crate::x86_64::layout::{KERNEL_START, MIN_RAM_BYTES};

#[test]
fn maps_writes_and_rejects_out_of_range_writes() {
    let mut ram = GuestRam::map(GuestLayout::new(MIN_RAM_BYTES).unwrap()).unwrap();
    ram.write(KERNEL_START, b"SOMA").unwrap();
    ram.write(MIN_RAM_BYTES - 4, b"SOMA").unwrap();
    assert!(ram.write(MIN_RAM_BYTES - 3, b"SOMA").is_err());
    assert!(ram.write(u64::MAX, b"S").is_err());
    assert_eq!(ram.layout().ram_bytes(), MIN_RAM_BYTES);
}

#[test]
fn shared_view_round_trips_and_checks_every_range() {
    let mut ram = GuestRam::map(GuestLayout::new(MIN_RAM_BYTES).unwrap()).unwrap();
    ram.write(KERNEL_START, &[1, 2, 3, 4]).unwrap();
    let shared = ram.shared();
    drop(ram);
    let mut buf = [0_u8; 4];
    shared
        .read_bytes(GuestAddress(KERNEL_START), &mut buf)
        .unwrap();
    assert_eq!(buf, [1, 2, 3, 4]);
    shared
        .write_obj_at(GuestAddress(KERNEL_START + 8), 0xdead_beef_u32)
        .unwrap();
    assert_eq!(
        shared
            .read_obj_at::<u32>(GuestAddress(KERNEL_START + 8))
            .unwrap(),
        0xdead_beef
    );
    assert!(shared.check_range(GuestAddress(0), MIN_RAM_BYTES).is_ok());
    assert!(shared.check_range(GuestAddress(1), MIN_RAM_BYTES).is_err());
    assert!(shared.check_range(GuestAddress(u64::MAX), 1).is_err());
    assert!(
        shared
            .read_bytes(GuestAddress(MIN_RAM_BYTES - 2), &mut buf)
            .is_err()
    );
    assert!(
        shared
            .write_bytes(GuestAddress(MIN_RAM_BYTES), &[0])
            .is_err()
    );
}
