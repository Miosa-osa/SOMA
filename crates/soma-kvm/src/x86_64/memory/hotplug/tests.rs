use super::*;
use crate::memory_layout::{GuestLayout, HOTPLUG_ALIGN_BYTES, MIN_RAM_BYTES};
use crate::virtio::{GuestAddress, GuestMemory as _};
use crate::x86_64::memory::GuestRam;

const REGION_BYTES: u64 = 2 * HOTPLUG_ALIGN_BYTES;
const BLOCK: u64 = 2 * 1024 * 1024;

fn region() -> HotplugRegion {
    HotplugRegion::new(GuestLayout::new(MIN_RAM_BYTES).unwrap(), REGION_BYTES).unwrap()
}

fn ram() -> Arc<HotplugRam> {
    Arc::new(HotplugRam::map(region()).unwrap())
}

fn backing(ram: &Arc<HotplugRam>) -> HotplugBacking {
    HotplugBacking::new(Arc::clone(ram))
}

#[test]
fn starts_resident_nowhere() {
    assert_eq!(ram().resident_bytes().unwrap(), 0);
}

#[test]
fn touched_pages_are_resident_and_discard_returns_them_and_zeroes_them() {
    let ram = ram();
    let start = region().guest_start;
    assert!(ram.write(start, &[0xa5; 4096]));
    assert!(ram.write(start + BLOCK, &[0xa5; 4096]));
    assert_eq!(ram.resident_bytes().unwrap(), 2 * 4096);

    backing(&ram).discard(0, BLOCK).unwrap();
    assert_eq!(ram.resident_bytes().unwrap(), 4096);
    let mut buf = [0xff_u8; 8];
    assert!(ram.read(start, &mut buf));
    assert_eq!(buf, [0; 8]);
    // The neighbouring block kept its content.
    assert!(ram.read(start + BLOCK, &mut buf));
    assert_eq!(buf, [0xa5; 8]);

    // Discarding the whole region, as UNPLUG_ALL does, drops the rest.
    backing(&ram).discard(0, REGION_BYTES).unwrap();
    assert_eq!(ram.resident_bytes().unwrap(), 0);
    // Discarding what is already gone is fine.
    backing(&ram).discard(0, REGION_BYTES).unwrap();
}

#[test]
fn discard_refuses_unaligned_empty_and_out_of_range_ranges() {
    let ram = ram();
    let mut backing = backing(&ram);
    assert!(ram.write(region().guest_start, &[1; 4096]));
    for (offset, len) in [
        (1, 4096),
        (0, 0),
        (0, 4097),
        (REGION_BYTES, 4096),
        (REGION_BYTES - 4096, 8192),
        (u64::MAX, 4096),
    ] {
        assert!(backing.discard(offset, len).is_err(), "{offset} {len}");
    }
    // A refused discard dropped nothing.
    assert_eq!(ram.resident_bytes().unwrap(), 4096);
}

#[test]
fn access_is_bounded_by_the_region() {
    let ram = ram();
    let region = region();
    let mut buf = [0_u8; 4];
    assert!(!ram.read(region.guest_start - 1, &mut buf));
    assert!(!ram.write(region.guest_end() - 3, &buf));
    assert!(ram.write(region.guest_end() - 4, &buf));
}

#[test]
fn the_shared_view_reaches_the_region_and_boot_ram_is_unaffected() {
    let mut guest = GuestRam::map(GuestLayout::new(MIN_RAM_BYTES).unwrap()).unwrap();
    let region = region();
    guest.attach_hotplug(region).unwrap();
    let shared = guest.shared();
    let base = region.guest_start;

    shared
        .write_bytes(GuestAddress(base + 16), &[1, 2, 3, 4])
        .unwrap();
    let mut buf = [0_u8; 4];
    shared
        .read_bytes(GuestAddress(base + 16), &mut buf)
        .unwrap();
    assert_eq!(buf, [1, 2, 3, 4]);
    assert!(shared.check_range(GuestAddress(base), REGION_BYTES).is_ok());
    assert!(
        shared
            .check_range(GuestAddress(base), REGION_BYTES + 1)
            .is_err()
    );
    assert!(shared.check_range(GuestAddress(base - 1), 2).is_err());
    shared.write_bytes(GuestAddress(0x1000), &[9]).unwrap();
    assert_eq!(guest.layout().ram_bytes(), MIN_RAM_BYTES);
    assert_eq!(guest.hotplug().unwrap().region(), region);
}

#[test]
fn a_machine_without_a_region_has_none() {
    let guest = GuestRam::map(GuestLayout::new(MIN_RAM_BYTES).unwrap()).unwrap();
    assert!(guest.hotplug().is_none());
    assert!(
        guest
            .shared()
            .check_range(GuestAddress(region().guest_start), 1)
            .is_err()
    );
}

#[test]
fn plug_accepts_whole_in_region_ranges_and_refuses_the_rest() {
    let ram = ram();
    let mut backing = backing(&ram);
    backing.plug(0, BLOCK).unwrap();
    backing.plug(REGION_BYTES - BLOCK, BLOCK).unwrap();
    for (offset, len) in [
        (1, 4096),
        (0, 0),
        (0, 4097),
        (REGION_BYTES, 4096),
        (REGION_BYTES - 4096, 8192),
        (u64::MAX, 4096),
        (4096, u64::MAX),
    ] {
        assert!(backing.plug(offset, len).is_err(), "{offset} {len}");
    }
}

#[test]
fn plug_touch_discard_returns_the_memory() {
    let ram = ram();
    let mut backing = backing(&ram);
    backing.plug(0, BLOCK).unwrap();
    assert!(ram.write(region().guest_start + 64, &[7; 4096]));
    assert_eq!(backing.resident_bytes().unwrap(), 2 * 4096);
    backing.discard(0, BLOCK).unwrap();
    assert_eq!(backing.resident_bytes().unwrap(), 0);
}
