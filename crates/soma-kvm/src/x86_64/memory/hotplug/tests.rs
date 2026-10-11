use super::*;
use crate::memory_layout::{GuestLayout, HOTPLUG_SLOT_BYTES, MIN_RAM_BYTES};
use crate::virtio::{GuestAddress, GuestMemory as _};
use crate::x86_64::memory::GuestRam;

fn region(slots: u64) -> HotplugRam {
    HotplugRam::map(HotplugGeometry::new(slots * HOTPLUG_SLOT_BYTES).unwrap()).unwrap()
}

fn start(region: &HotplugRam) -> u64 {
    region.geometry().guest_start()
}

/// Resident bytes of the first `len` bytes of the region, through `mincore`.
#[allow(unsafe_code)]
fn resident_pages(region: &HotplugRam, len: usize) -> usize {
    let pages = len / 4096;
    let mut vec = vec![0_u8; pages];
    // SAFETY: the mapping is live for the call and `vec` holds one byte per page of `len`.
    let rc = unsafe {
        libc::mincore(
            region.mapping.host_address().unwrap() as *mut libc::c_void,
            len,
            vec.as_mut_ptr().cast(),
        )
    };
    assert_eq!(rc, 0);
    vec.iter().filter(|byte| **byte & 1 == 1).count()
}

#[test]
fn starts_empty_and_resident_nowhere() {
    let region = region(1);
    assert_eq!(region.plugged_bytes(), 0);
    assert!(!region.is_plugged(start(&region), 1));
    assert_eq!(resident_pages(&region, 4096 * 512), 0);
}

#[test]
fn an_unplugged_block_is_not_addressable() {
    let region = region(1);
    let mut buf = [0_u8; 4];
    assert!(!region.read(start(&region), &mut buf));
    assert!(!region.write(start(&region), &[1]));
}

#[test]
fn plug_makes_blocks_addressable_and_counts_them() {
    let region = region(1);
    region
        .plug(start(&region), 2 * HOTPLUG_BLOCK_BYTES)
        .unwrap();
    assert_eq!(region.plugged_bytes(), 2 * HOTPLUG_BLOCK_BYTES);
    assert!(region.write(start(&region) + 8, b"SOMA"));
    let mut buf = [0_u8; 4];
    assert!(region.read(start(&region) + 8, &mut buf));
    assert_eq!(&buf, b"SOMA");
    // One byte past the plugged blocks is still unplugged.
    assert!(!region.read(start(&region) + 2 * HOTPLUG_BLOCK_BYTES, &mut buf));
    // An access straddling plugged and unplugged blocks is refused whole.
    assert!(!region.write(start(&region) + 2 * HOTPLUG_BLOCK_BYTES - 2, b"SOMA"));
}

#[test]
fn plug_and_unplug_refuse_bad_ranges_and_wrong_state() {
    let region = region(1);
    let base = start(&region);
    assert!(matches!(
        region.plug(base + 1, HOTPLUG_BLOCK_BYTES),
        Err(HotplugRamError::Range(_))
    ));
    assert!(matches!(
        region.plug(base, HOTPLUG_SLOT_BYTES + HOTPLUG_BLOCK_BYTES),
        Err(HotplugRamError::Range(_))
    ));
    assert_eq!(
        region.unplug(base, HOTPLUG_BLOCK_BYTES),
        Err(HotplugRamError::NotPlugged { block: 0 })
    );
    region.plug(base, HOTPLUG_BLOCK_BYTES).unwrap();
    assert_eq!(
        region.plug(base, 2 * HOTPLUG_BLOCK_BYTES),
        Err(HotplugRamError::AlreadyPlugged { block: 0 })
    );
    // All or none: the refused plug of blocks 0 and 1 did not plug block 1.
    assert_eq!(region.plugged_bytes(), HOTPLUG_BLOCK_BYTES);
    assert_eq!(
        region.unplug(base, 2 * HOTPLUG_BLOCK_BYTES),
        Err(HotplugRamError::NotPlugged { block: 1 })
    );
    assert_eq!(region.plugged_bytes(), HOTPLUG_BLOCK_BYTES);
}

#[test]
fn unplug_returns_the_memory_and_a_replug_reads_zero() {
    let region = region(1);
    let base = start(&region);
    region.plug(base, 2 * HOTPLUG_BLOCK_BYTES).unwrap();
    let page = vec![0xa5_u8; 4096];
    assert!(region.write(base, &page));
    assert!(region.write(base + HOTPLUG_BLOCK_BYTES, &page));
    assert!(resident_pages(&region, 2 * 2 * 1024 * 1024) >= 2);

    region.unplug(base, 2 * HOTPLUG_BLOCK_BYTES).unwrap();
    assert_eq!(region.plugged_bytes(), 0);
    assert_eq!(resident_pages(&region, 2 * 2 * 1024 * 1024), 0);
    let mut buf = [0xff_u8; 8];
    assert!(!region.read(base, &mut buf));

    region.plug(base, HOTPLUG_BLOCK_BYTES).unwrap();
    assert!(region.read(base, &mut buf));
    assert_eq!(buf, [0; 8]);
}

#[test]
fn the_shared_view_reaches_plugged_blocks_only() {
    let mut ram = GuestRam::map(GuestLayout::new(MIN_RAM_BYTES).unwrap()).unwrap();
    let geometry = HotplugGeometry::new(HOTPLUG_SLOT_BYTES).unwrap();
    ram.attach_hotplug(geometry).unwrap();
    let hotplug = ram.hotplug().unwrap();
    let shared = ram.shared();
    let base = geometry.guest_start();

    assert!(shared.check_range(GuestAddress(base), 1).is_err());
    assert!(
        shared
            .write_bytes(GuestAddress(base), &[1, 2, 3, 4])
            .is_err()
    );

    hotplug.plug(base, HOTPLUG_BLOCK_BYTES).unwrap();
    shared
        .write_bytes(GuestAddress(base + 16), &[1, 2, 3, 4])
        .unwrap();
    let mut buf = [0_u8; 4];
    shared
        .read_bytes(GuestAddress(base + 16), &mut buf)
        .unwrap();
    assert_eq!(buf, [1, 2, 3, 4]);
    assert!(
        shared
            .check_range(GuestAddress(base), HOTPLUG_BLOCK_BYTES)
            .is_ok()
    );
    assert!(
        shared
            .check_range(GuestAddress(base), HOTPLUG_BLOCK_BYTES + 1)
            .is_err()
    );

    hotplug.unplug(base, HOTPLUG_BLOCK_BYTES).unwrap();
    assert!(
        shared
            .read_bytes(GuestAddress(base + 16), &mut buf)
            .is_err()
    );
    // Boot RAM is unaffected by the region.
    shared.write_bytes(GuestAddress(0x1000), &[9]).unwrap();
    assert_eq!(ram.layout().ram_bytes(), MIN_RAM_BYTES);
}

#[test]
fn a_machine_without_a_region_has_none() {
    let ram = GuestRam::map(GuestLayout::new(MIN_RAM_BYTES).unwrap()).unwrap();
    assert!(ram.hotplug().is_none());
    let shared = ram.shared();
    assert!(
        shared
            .check_range(GuestAddress(crate::memory_layout::HOTPLUG_REGION_START), 1)
            .is_err()
    );
}
