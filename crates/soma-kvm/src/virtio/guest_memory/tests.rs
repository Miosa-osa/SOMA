use super::*;

const KIB: usize = 1024;

fn two_regions() -> VecGuestMemory {
    VecGuestMemory::new(&[
        (GuestAddress(0), 4 * KIB),
        (GuestAddress(0x1_0000), 2 * KIB),
    ])
    .expect("valid layout")
}

#[test]
fn rejects_empty_overlapping_and_overflowing_regions() {
    assert_eq!(
        VecGuestMemory::new(&[(GuestAddress(0), 0)]).err(),
        Some(RegionLayoutError::EmptyRegion)
    );
    assert_eq!(
        VecGuestMemory::new(&[(GuestAddress(0), KIB), (GuestAddress(KIB as u64 - 1), KIB)]).err(),
        Some(RegionLayoutError::Overlap)
    );
    assert_eq!(
        VecGuestMemory::new(&[(GuestAddress(KIB as u64), KIB), (GuestAddress(0), KIB)]).err(),
        Some(RegionLayoutError::Overlap)
    );
    assert_eq!(
        VecGuestMemory::new(&[(GuestAddress(u64::MAX), KIB)]).err(),
        Some(RegionLayoutError::Overflow)
    );
}

#[test]
fn round_trips_little_endian_values_inside_a_region() {
    let mem = two_regions();
    mem.write_obj_at(GuestAddress(0x10), 0x1122_3344_5566_7788u64)
        .expect("write");
    assert_eq!(
        mem.read_obj_at::<u64>(GuestAddress(0x10)).expect("read"),
        0x1122_3344_5566_7788
    );
    assert_eq!(
        mem.read_obj_at::<u16>(GuestAddress(0x10)).expect("read"),
        0x7788
    );
    assert_eq!(
        mem.read_obj_at::<u8>(GuestAddress(0x17)).expect("read"),
        0x11
    );
    let mut raw = [0u8; 4];
    mem.read_bytes(GuestAddress(0x10), &mut raw).expect("read");
    assert_eq!(raw, [0x88, 0x77, 0x66, 0x55]);
}

#[test]
fn accepts_range_ending_exactly_at_region_end() {
    let mem = two_regions();
    mem.check_range(GuestAddress(0x1_0000 + 2 * KIB as u64 - 8), 8)
        .expect("end-inclusive fit");
    mem.check_range(GuestAddress(4 * KIB as u64 - 1), 1)
        .expect("last byte");
    mem.check_range(GuestAddress(0), 0).expect("zero length");
}

#[test]
fn rejects_ranges_outside_or_spanning_regions() {
    let mem = two_regions();
    let straddle = GuestAddress(4 * KIB as u64 - 4);
    assert_eq!(
        mem.check_range(straddle, 8),
        Err(GuestMemoryError::OutOfRegion {
            addr: straddle,
            len: 8
        })
    );
    let gap = GuestAddress(0x8000);
    assert_eq!(
        mem.read_obj_at::<u32>(gap),
        Err(GuestMemoryError::OutOfRegion { addr: gap, len: 4 })
    );
    let spanning = GuestAddress(0x1000 - 2);
    assert_eq!(
        mem.write_bytes(spanning, &vec![0; 0x1_0000]),
        Err(GuestMemoryError::OutOfRegion {
            addr: spanning,
            len: 0x1_0000
        })
    );
}

#[test]
fn rejects_address_plus_length_overflow_before_lookup() {
    let mem = two_regions();
    let addr = GuestAddress(u64::MAX - 2);
    assert_eq!(
        mem.check_range(addr, 4),
        Err(GuestMemoryError::Overflow { addr, len: 4 })
    );
    assert_eq!(
        mem.read_obj_at::<u64>(GuestAddress(u64::MAX)),
        Err(GuestMemoryError::Overflow {
            addr: GuestAddress(u64::MAX),
            len: 8
        })
    );
}

#[test]
fn guest_address_helpers_are_checked() {
    assert_eq!(GuestAddress(u64::MAX).checked_add(1), None);
    assert_eq!(GuestAddress(8).checked_add(8), Some(GuestAddress(16)));
    assert!(GuestAddress(32).is_aligned(16));
    assert!(!GuestAddress(30).is_aligned(16));
    assert!(!GuestAddress(0).is_aligned(0));
    assert_eq!(GuestAddress(7).raw(), 7);
}

#[test]
fn errors_display_without_guest_bytes() {
    let text = GuestMemoryError::OutOfRegion {
        addr: GuestAddress(0x10),
        len: 4,
    }
    .to_string();
    assert_eq!(text, "guest range 0x10+0x4 is unregistered");
    assert_eq!(
        RegionLayoutError::Overlap.to_string(),
        "guest memory regions overlap"
    );
}
