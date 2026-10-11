//! The memory slot on the bus: declared, routed, serviced, captured, and restored.

use super::super::slots::SlotRestoreError;
use super::super::*;
use super::{SPEC_COMMAND_LINE, devices, init_slot};
use crate::virtio::devices::harness::{GuestRig, Seg};
use crate::virtio::devices::mem::backing::RecordingBacking;
use crate::virtio::devices::mem::{BLOCK_SIZE, MEM_FEATURES, MemDevice};
use crate::virtio::guest_memory::GuestAddress;
use crate::virtio::transport::registers::{AccessWidth, REG_DEVICE_ID};

const W: AccessWidth = AccessWidth::U32;
const REGION: u64 = 0x1_0000_0000;
const BLOCKS: u64 = 8;

struct Source(Vec<(u64, Slot, u16)>);

impl NotifySource for Source {
    type Error = ();
    fn register(&mut self, addr: GuestAddress, slot: Slot, queue: u16) -> Result<(), ()> {
        self.0.push((addr.raw(), slot, queue));
        Ok(())
    }
}

fn mem_device() -> MemDevice {
    MemDevice::new(
        REGION,
        BLOCKS * BLOCK_SIZE,
        Box::new(RecordingBacking::default()),
    )
    .expect("device")
}

fn models() -> BusDevices {
    let mut models = devices([2, 0, 0, 0, 0, 1]);
    models.mem = Some(mem_device());
    models
}

fn mem_bus() -> MmioBus {
    MmioBus::new(models()).expect("bus")
}

#[test]
fn the_memory_slot_is_declared_only_when_asked_for() {
    assert!(!DeviceSet::FULL.has(Slot::Mem));
    assert!(!DeviceSet::new(true, true).has(Slot::Mem));
    let with = DeviceSet::FULL.with_mem(true);
    assert!(with.has(Slot::Mem) && with.mem());
    assert_ne!(with, DeviceSet::FULL);
    assert_eq!(
        kernel_command_line(with),
        format!("{SPEC_COMMAND_LINE} virtio_mmio.device=4K@0xd0005000:10:5")
    );
    assert_eq!(kernel_command_line(DeviceSet::FULL), SPEC_COMMAND_LINE);
    assert_eq!(Slot::Mem.base(), 0xd000_5000);
    assert_eq!(Slot::Mem.gsi(), 10);
    assert_eq!(Slot::Mem.notify_addr(), 0xd000_5050);
    assert_eq!(Slot::Mem.device_id(), 24);
}

#[test]
fn a_bus_with_the_memory_device_reports_it_and_routes_to_it() {
    let mut bus = mem_bus();
    assert_eq!(bus.device_set(), DeviceSet::FULL.with_mem(true));
    assert_eq!(
        bus.dispatch_read(Slot::Mem.base() + REG_DEVICE_ID, W),
        Ok(24)
    );
    assert_eq!(bus.interrupt_status(Slot::Mem), 0);
    assert!(bus.mem().is_some());

    let mut source = Source(Vec::new());
    bus.register_notify_sources(&mut source).expect("register");
    assert!(source.0.contains(&(0xd000_5050, Slot::Mem, 0)));
    assert_eq!(
        source.0.len(),
        6 + 3,
        "every present queue, the memory queue included"
    );
}

#[test]
fn a_bus_without_the_memory_device_leaves_its_page_unmapped() {
    let mut bus = MmioBus::new(devices([0; 6])).expect("bus");
    assert!(bus.mem().is_none());
    assert_eq!(
        bus.dispatch_read(Slot::Mem.base(), W),
        Err(BusViolation::UnmappedAddress {
            gpa: Slot::Mem.base()
        })
    );
    assert_eq!(bus.snapshot_all().len(), 5);
    let rig = GuestRig::new(&[16]);
    assert_eq!(
        bus.service(Slot::Mem, 0, &rig.mem, 8).map(|r| r.completed),
        Ok(0)
    );
}

#[test]
fn the_guest_plugs_a_block_through_the_bus_and_the_host_sees_it() {
    let mut bus = mem_bus();
    let mut rig = GuestRig::new(&[16]);
    init_slot(&mut bus, &rig, Slot::Mem, MEM_FEATURES);
    let transport = bus.mem_mut().expect("mem");
    transport
        .device_mut()
        .set_requested_size(2 * BLOCK_SIZE)
        .expect("size");
    transport.signal_config_change();
    assert!(bus.interrupt_pending(Slot::Mem));

    let mut request = [0_u8; 24];
    request[8..16].copy_from_slice(&REGION.to_le_bytes());
    request[16..18].copy_from_slice(&2_u16.to_le_bytes());
    let body = rig.alloc(&request);
    let reply = rig.alloc_zeroed(10);
    rig.submit(0, &[Seg::readable(body, 24), Seg::writable(reply, 10)]);
    let report = bus.service(Slot::Mem, 0, &rig.mem, 8).expect("service");
    assert_eq!(report.completed, 1);
    assert_eq!(rig.read(reply, 2), [0, 0], "ACK");
    assert_eq!(
        bus.mem().expect("mem").device().plugged_size(),
        2 * BLOCK_SIZE
    );
}

#[test]
fn an_empty_memory_slot_is_captured_and_restored_with_the_rest() {
    let mut bus = mem_bus();
    let rig = GuestRig::new(&[16]);
    init_slot(&mut bus, &rig, Slot::Mem, MEM_FEATURES);
    let records = bus.snapshot_all();
    assert_eq!(records.len(), 6);
    assert_eq!(records.last().map(|record| record.slot), Some(Slot::Mem));
    let restored = MmioBus::restore(models(), &records, &rig.mem).expect("restore");
    assert_eq!(restored.device_set(), DeviceSet::FULL.with_mem(true));
    assert_eq!(restored.mem().expect("mem").device().plugged_size(), 0);
}

#[test]
fn a_snapshot_from_a_machine_with_a_different_memory_declaration_is_refused() {
    let mut with = mem_bus();
    let rig = GuestRig::new(&[16]);
    let full = with.snapshot_all();
    assert_eq!(
        MmioBus::restore(devices([2, 0, 0, 0, 0, 1]), &full, &rig.mem).err(),
        Some(SlotRestoreError::SlotSetMismatch {
            expected: 5,
            actual: 6
        })
    );
    let mut without = MmioBus::new(devices([2, 0, 0, 0, 0, 1])).expect("bus");
    let five = without.snapshot_all();
    assert_eq!(
        MmioBus::restore(models(), &five, &rig.mem).err(),
        Some(SlotRestoreError::SlotSetMismatch {
            expected: 6,
            actual: 5
        })
    );
}
