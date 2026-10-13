//! The device builders' own tests: geometry, identity, role checks and the volume slot.

use super::*;
use crate::virtio::{Slot, VIRTIO_BLK_F_RO};

fn image(bytes: u64) -> File {
    let file = tempfile::tempfile().unwrap();
    file.set_len(bytes).unwrap();
    file
}

fn identity() -> DeviceIdentity {
    DeviceIdentity {
        guest_cid: 3,
        guest_mac: [0x02, 0x53, 0x4f, 0x4d, 0x41, 0x01],
    }
}

fn full_disks() -> SandboxDisks {
    SandboxDisks {
        root: image(64 * 4096),
        overlay: Some(image(64 * 1024 * 1024)),
        volume: None,
    }
}

#[test]
fn builds_the_five_slots_with_fixed_geometry_and_identity() {
    let bus = build_bus(
        full_disks(),
        identity(),
        DeviceSet::FULL,
        MachineContract::V1,
        None,
    )
    .unwrap();
    assert_eq!(bus.root().device().role(), BlockRole::ImmutableRoot);
    assert_eq!(bus.root().device().blk_size(), BLOCK_SIZE);
    assert_eq!(bus.root().device().capacity_sectors(), 64 * 8);
    assert_ne!(bus.root().device().feature_allowlist_ro(), 0);
    assert_eq!(
        bus.overlay().unwrap().device().role(),
        BlockRole::PrivateOverlay
    );
    assert_eq!(bus.vsock().device().guest_cid(), 3);
    assert_eq!(bus.net().unwrap().device().mac(), identity().guest_mac);
    assert!(!bus.net().unwrap().device().link_up());
    assert_eq!(Slot::ALL.len(), 6);
    assert!(bus.mem().is_none());
}

fn region() -> (u64, u64, Box<dyn RegionBacking + Send>) {
    let backing: Box<dyn RegionBacking + Send> =
        Box::new(crate::virtio::RecordingBacking::default());
    (0x20_0000_0000, 64 * 2 * 1024 * 1024, backing)
}

#[test]
fn a_declared_memory_device_is_built_over_the_region_the_machine_placed() {
    let bus = build_bus(
        full_disks(),
        identity(),
        DeviceSet::FULL_WITH_MEM,
        MachineContract::V2,
        Some(region()),
    )
    .unwrap();
    assert_eq!(bus.device_set(), DeviceSet::FULL_WITH_MEM);
    let mem = bus.mem().expect("the memory slot").device();
    assert_eq!(mem.region_addr(), 0x20_0000_0000);
    assert_eq!(mem.region_size(), 64 * 2 * 1024 * 1024);
    assert_eq!(mem.plugged_size(), 0);
}

#[test]
fn a_device_set_and_a_region_that_disagree_are_refused_both_ways() {
    let declared_without_region = build_bus(
        full_disks(),
        identity(),
        DeviceSet::FULL_WITH_MEM,
        MachineContract::V2,
        None,
    );
    assert!(declared_without_region.is_err());
    let region_without_declaration = build_bus(
        full_disks(),
        identity(),
        DeviceSet::FULL,
        MachineContract::V2,
        Some(region()),
    );
    assert!(region_without_declaration.is_err());
}

#[test]
fn builds_only_the_declared_devices() {
    let disks = SandboxDisks {
        root: image(64 * 4096),
        overlay: None,
        volume: None,
    };
    let bus = build_bus(
        disks,
        identity(),
        DeviceSet::new(false, false),
        MachineContract::V1,
        None,
    )
    .unwrap();
    assert!(bus.overlay().is_none());
    assert!(bus.net().is_none());
    assert_eq!(bus.device_set(), DeviceSet::new(false, false));
    assert_eq!(bus.device_set().present().count(), 3);
}

#[test]
fn refuses_a_head_the_declared_set_has_no_slot_for() {
    let Err(error) = build_bus(
        full_disks(),
        identity(),
        DeviceSet::new(false, true),
        MachineContract::V1,
        None,
    ) else {
        panic!("an overlay handed to a machine with no overlay slot must be refused");
    };
    assert_eq!(error.phase(), Phase::Devices);
    let disks = SandboxDisks {
        root: image(64 * 4096),
        overlay: None,
        volume: None,
    };
    let Err(error) = build_bus(
        disks,
        identity(),
        DeviceSet::FULL,
        MachineContract::V2,
        None,
    ) else {
        panic!("a declared overlay with no head must be refused");
    };
    assert_eq!(error.phase(), Phase::Devices);
}

#[test]
fn rejects_a_reserved_cid_and_an_unaligned_root() {
    let Err(error) = build_bus(
        full_disks(),
        DeviceIdentity {
            guest_cid: 2,
            ..identity()
        },
        DeviceSet::FULL,
        MachineContract::V1,
        None,
    ) else {
        panic!("a reserved CID must be rejected");
    };
    assert_eq!(error.phase(), Phase::Devices);
    assert!(matches!(error.kind(), MachineErrorKind::Vsock(_)));
    let disks = SandboxDisks {
        root: image(4096 + 512),
        overlay: Some(image(64 * 1024 * 1024)),
        volume: None,
    };
    let Err(error) = build_bus(
        disks,
        identity(),
        DeviceSet::FULL,
        MachineContract::V2,
        None,
    ) else {
        panic!("an unaligned root must be rejected");
    };
    assert!(matches!(error.kind(), MachineErrorKind::Block(_)));
}

impl BlockDevice {
    fn feature_allowlist_ro(&self) -> u64 {
        self.features() & VIRTIO_BLK_F_RO
    }
}
