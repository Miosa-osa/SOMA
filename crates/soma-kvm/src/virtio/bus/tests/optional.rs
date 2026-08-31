//! What a bus built without its optional slots does, and refuses to do.

use super::super::slots::SlotRestoreError;
use super::super::*;
use super::{bus, devices, init_slot};
use crate::virtio::devices::harness::GuestRig;
use crate::virtio::devices::net::NET_FEATURES;
use crate::virtio::guest_memory::GuestAddress;
use crate::virtio::transport::registers::AccessWidth;

const W: AccessWidth = AccessWidth::U32;

#[test]
fn a_bus_without_the_optional_slots_registers_and_captures_only_what_it_has() {
    struct Source(Vec<(u64, Slot, u16)>);
    impl NotifySource for Source {
        type Error = ();
        fn register(&mut self, addr: GuestAddress, slot: Slot, queue: u16) -> Result<(), ()> {
            self.0.push((addr.raw(), slot, queue));
            Ok(())
        }
    }
    let mut models = devices([0; 6]);
    models.overlay = None;
    models.net = None;
    let mut bus = MmioBus::new(models).expect("bus");
    assert_eq!(bus.device_set(), DeviceSet::new(false, false));

    let mut source = Source(Vec::new());
    bus.register_notify_sources(&mut source).expect("register");
    assert_eq!(
        source.0.len(),
        5,
        "no ioeventfd for a device that is not there"
    );
    assert!(source.0.iter().all(|(_, slot, _)| *slot != Slot::Overlay));

    // The reserved page answers nothing rather than answering as a device.
    assert_eq!(
        bus.dispatch_read(Slot::Overlay.base(), W),
        Err(BusViolation::UnmappedAddress {
            gpa: Slot::Overlay.base()
        })
    );
    assert_eq!(bus.interrupt_status(Slot::Net), 0);
    assert!(bus.snapshot(Slot::Overlay).is_none());
    assert_eq!(bus.snapshot_all().len(), 3);
}

#[test]
fn a_snapshot_of_a_different_device_set_is_refused() {
    let mut bus = bus();
    let rig = GuestRig::new(&[16, 16]);
    init_slot(&mut bus, &rig, Slot::Net, NET_FEATURES);
    let full = bus.snapshot_all();
    let mut fewer = devices([0; 6]);
    fewer.overlay = None;
    fewer.net = None;
    assert_eq!(
        MmioBus::restore(fewer, &full, &rig.mem).err(),
        Some(SlotRestoreError::SlotSetMismatch {
            expected: 3,
            actual: 5
        })
    );
}
