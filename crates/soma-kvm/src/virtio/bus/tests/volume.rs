//! The durable volume slot on the bus: its page, its interrupt, and that it moves nothing else.

use super::super::*;
use crate::virtio::transport::registers::REG_QUEUE_NOTIFY;

#[test]
fn the_volume_slot_follows_the_memory_device_and_moves_no_earlier_slot() {
    assert_eq!(Slot::Volume.index(), 6);
    assert_eq!(Slot::Volume.base(), 0xd000_6000);
    assert_eq!(Slot::Volume.end(), 0xd000_6fff);
    assert_eq!(Slot::Volume.gsi(), 11);
    assert_eq!(Slot::Volume.device_id(), 2);
    assert_eq!(Slot::Volume.queue_count(), 1);
    assert_eq!(SLOT_COUNT, 7);

    // Every earlier slot keeps the address and interrupt it always had, so a Generation that
    // never declares a volume is byte for byte the machine it was.
    assert_eq!(Slot::Root.base(), 0xd000_0000);
    assert_eq!(Slot::Rng.base(), 0xd000_4000);
    assert_eq!(Slot::Mem.base(), 0xd000_5000);
    assert_eq!(Slot::Mem.gsi(), 10);
    assert_eq!(
        Slot::from_gpa(Slot::Volume.notify_addr()),
        Some((Slot::Volume, REG_QUEUE_NOTIFY))
    );
}

#[test]
fn a_volume_is_declared_only_when_it_is_asked_for() {
    let without = DeviceSet::new(true, false);
    assert!(!without.volume());
    let with = without.with_volume(true);
    assert!(with.volume());

    // The certified defaults never carry it.
    assert!(!DeviceSet::FULL.volume());
    assert!(!DeviceSet::FULL_WITH_MEM.volume());

    // It is presented last, after the memory device, and only then is it declared on the line.
    let presented: Vec<Slot> = with.with_mem(true).present().collect();
    assert_eq!(presented.last(), Some(&Slot::Volume));
    assert!(kernel_command_line(with).contains("4K@0xd0006000:11:6"));
    assert!(!kernel_command_line(without).contains("0xd0006000"));
}
