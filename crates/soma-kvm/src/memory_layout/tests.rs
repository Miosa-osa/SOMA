use super::*;
use crate::virtio::{MMIO_PAGE_SIZE, MMIO_WINDOW_BASE};

fn mib(count: u64) -> u64 {
    count * 1024 * 1024
}

#[test]
fn accepts_the_range_and_rejects_the_rest() {
    assert!(GuestLayout::new(MIN_RAM_BYTES).is_ok());
    assert!(GuestLayout::new(MAX_RAM_BYTES).is_ok());
    assert_eq!(
        GuestLayout::new(MIN_RAM_BYTES - PAGE_SIZE),
        Err(LayoutError::BelowMinimum {
            bytes: MIN_RAM_BYTES - PAGE_SIZE
        })
    );
    assert_eq!(
        GuestLayout::new(MAX_RAM_BYTES + PAGE_SIZE),
        Err(LayoutError::AboveMaximum {
            bytes: MAX_RAM_BYTES + PAGE_SIZE
        })
    );
    assert_eq!(
        GuestLayout::new(MIN_RAM_BYTES + 1),
        Err(LayoutError::NotPageAligned {
            bytes: MIN_RAM_BYTES + 1
        })
    );
    assert!(GuestLayout::new(0).is_err());
    assert!(GuestLayout::new(u64::MAX).is_err());
}

#[test]
fn ram_at_or_below_the_boundary_is_one_range_from_zero() {
    for size in [MIN_RAM_BYTES, mib(1024), LOW_RAM_END] {
        let layout = GuestLayout::new(size).unwrap();
        assert_eq!(layout.ram_bytes(), size);
        assert!(!layout.is_split());
        assert_eq!(
            layout.regions(),
            [RamRegion {
                slot: 0,
                guest_start: 0,
                host_offset: 0,
                size
            }]
        );
        // Every address maps to itself, which is what makes a version 1 snapshot byte-identical.
        assert_eq!(
            layout.host_offset(HIGH_MEMORY_START),
            Some(HIGH_MEMORY_START)
        );
        assert_eq!(layout.host_offset(size - 1), Some(size - 1));
        assert_eq!(layout.host_offset(size), None);
        assert_eq!(layout.memory_map().len(), 3);
    }
}

#[test]
fn ram_above_the_boundary_splits_around_the_mmio_hole() {
    let layout = GuestLayout::new(MAX_RAM_BYTES).unwrap();
    assert_eq!(layout.ram_bytes(), MAX_RAM_BYTES);
    assert!(layout.is_split());
    assert_eq!(
        layout.regions(),
        [
            RamRegion {
                slot: 0,
                guest_start: 0,
                host_offset: 0,
                size: LOW_RAM_END,
            },
            RamRegion {
                slot: HIGH_RAM_SLOT,
                guest_start: MMIO_HOLE_END,
                host_offset: LOW_RAM_END,
                size: MAX_RAM_BYTES - LOW_RAM_END,
            },
        ]
    );
    // The ranges tile the object without a gap and cover exactly the RAM the caller asked for.
    let total: u64 = layout.regions().iter().map(|region| region.size).sum();
    assert_eq!(total, MAX_RAM_BYTES);
    assert_eq!(
        layout.regions()[0].host_end(),
        layout.regions()[1].host_offset
    );
}

#[test]
fn the_high_range_skips_the_launch_page_slot() {
    let split = GuestLayout::new(MAX_RAM_BYTES).unwrap();
    assert_eq!(split.regions()[0].slot, 0);
    assert_eq!(split.regions()[1].slot, HIGH_RAM_SLOT);
    assert_ne!(split.regions()[1].slot, 1);
    // A single-range machine keeps the version 1 numbering exactly.
    assert_eq!(GuestLayout::new(mib(512)).unwrap().regions()[0].slot, 0);
}

#[test]
fn the_mmio_hole_is_reserved_and_has_no_object_bytes() {
    let layout = GuestLayout::new(MAX_RAM_BYTES).unwrap();
    assert_eq!(layout.host_offset(LOW_RAM_END), None);
    assert_eq!(layout.host_offset(MMIO_HOLE_END - 1), None);
    assert_eq!(layout.host_offset(LOW_RAM_END - 1), Some(LOW_RAM_END - 1));
    assert_eq!(layout.host_offset(MMIO_HOLE_END), Some(LOW_RAM_END));
    assert!(!layout.contains(LOW_RAM_END, 1));
    assert!(!layout.contains(LOW_RAM_END - PAGE_SIZE, PAGE_SIZE + 1));
    assert!(layout.contains(LOW_RAM_END - PAGE_SIZE, PAGE_SIZE));
    assert!(layout.contains(MMIO_HOLE_END, PAGE_SIZE));
    let map = layout.memory_map();
    assert_eq!(map.len(), 5);
    assert_eq!(map[3].kind, MemoryKind::Reserved);
    assert_eq!(map[3].address, LOW_RAM_END);
    assert_eq!(map[3].size, MMIO_HOLE_BYTES);
    assert_eq!(map[4].kind, MemoryKind::Ram);
    assert_eq!(map[4].address, MMIO_HOLE_END);
    // Every byte of the object is reachable and nothing outside it is.
    // The object is the whole RAM size, but the report marks the legacy hole reserved, so the
    // RAM entries cover the object less that hole: the same relationship version 1 already has.
    let covered: u64 = map
        .iter()
        .filter(|entry| entry.kind == MemoryKind::Ram)
        .map(|entry| entry.size)
        .sum();
    assert_eq!(
        covered,
        MAX_RAM_BYTES - (HIGH_MEMORY_START - LEGACY_HOLE_START)
    );
    let reserved: u64 = map
        .iter()
        .filter(|entry| entry.kind == MemoryKind::Reserved)
        .map(|entry| entry.size)
        .sum();
    assert_eq!(covered + reserved, MAX_RAM_BYTES + MMIO_HOLE_BYTES);
    assert_eq!(map[0].size + map[1].size, HIGH_MEMORY_START);
}

#[test]
fn the_fixed_mmio_window_lies_inside_the_reserved_hole() {
    let layout = GuestLayout::new(MAX_RAM_BYTES).unwrap();
    assert_eq!(layout.host_offset(MMIO_WINDOW_BASE), None);
    assert!(!layout.contains(MMIO_WINDOW_BASE, MMIO_PAGE_SIZE));
    // A smaller machine must never place its device window inside RAM either.
    let small = GuestLayout::new(LOW_RAM_END).unwrap();
    assert_eq!(small.host_offset(MMIO_WINDOW_BASE), None);
}

#[test]
fn containment_uses_checked_arithmetic() {
    let layout = GuestLayout::new(mib(512)).unwrap();
    assert!(layout.contains(KERNEL_START, 16));
    assert!(layout.contains(mib(512) - 1, 1));
    assert!(!layout.contains(mib(512), 1));
    assert!(!layout.contains(u64::MAX, 1));
    assert!(!layout.contains(0, 0));
    assert_eq!(layout.host_offset(u64::MAX), None);
}
