//! Table, dispatch, routing, seam, and snapshot tests for the bus.

#[path = "tests/optional.rs"]
mod optional;

use super::slots::{SlotRestoreError, SlotSnapshot};
use super::*;
use crate::virtio::device::DeviceStateError;
use crate::virtio::devices::block::backend::MemoryBackend;
use crate::virtio::devices::harness::{GuestRig, Seg};
use crate::virtio::devices::net::NET_FEATURES;
use crate::virtio::devices::net::backend::LoopbackBackend;
use crate::virtio::devices::rng::RNG_FEATURES;
use crate::virtio::devices::rng::backend::CounterEntropy;
use crate::virtio::transport::TransportEvent;
use crate::virtio::transport::registers::*;

const W: AccessWidth = AccessWidth::U32;
const CID: u64 = 1234;

struct Sink(Vec<u32>);
impl IrqSink for Sink {
    type Error = ();
    fn signal(&mut self, gsi: u32) -> Result<(), ()> {
        self.0.push(gsi);
        Ok(())
    }
}

const SPEC_COMMAND_LINE: &str = "virtio_mmio.device=4K@0xd0000000:5:0 virtio_mmio.device=4K@0xd0001000:6:1 virtio_mmio.device=4K@0xd0002000:7:2 virtio_mmio.device=4K@0xd0003000:8:3 virtio_mmio.device=4K@0xd0004000:9:4";

fn block(role: BlockRole) -> BlockDevice {
    let backend = MemoryBackend::zeroed(8, role == BlockRole::ImmutableRoot);
    BlockDevice::new(role, Box::new(backend), 512, [0; 20]).expect("block")
}

fn devices(mac: [u8; 6]) -> BusDevices {
    BusDevices {
        root: block(BlockRole::ImmutableRoot),
        overlay: Some(block(BlockRole::PrivateOverlay)),
        net: Some(NetDevice::new(Box::new(LoopbackBackend::default()), mac)),
        vsock: VsockDevice::new(CID).expect("vsock"),
        rng: RngDevice::new(Box::new(CounterEntropy::default())),
    }
}

fn bus() -> MmioBus {
    MmioBus::new(devices([2, 0, 0, 0, 0, 1])).expect("bus")
}

/// Runs the model driver's init sequence for `slot` through the bus.
fn init_slot(bus: &mut MmioBus, rig: &GuestRig, slot: Slot, features: u64) {
    let mut shim =
        MmioTransport::new(RngDevice::new(Box::new(CounterEntropy::default()))).expect("shim");
    let _ = &mut shim;
    let base = slot.base();
    let mut w = |offset: u64, value: u64| {
        bus.dispatch_write(base + offset, W, value, &rig.mem)
            .unwrap_or_else(|violation| panic!("write {offset:#x}: {violation}"))
            .event
    };
    assert_eq!(w(REG_STATUS, 0), TransportEvent::Reset);
    w(REG_STATUS, 1);
    w(REG_STATUS, 3);
    w(REG_DRIVER_FEATURES_SEL, 0);
    w(REG_DRIVER_FEATURES, features & 0xffff_ffff);
    w(REG_DRIVER_FEATURES_SEL, 1);
    w(REG_DRIVER_FEATURES, features >> 32);
    w(REG_STATUS, 11);
    for queue in 0..slot.queue_count() {
        w(REG_QUEUE_SEL, u64::from(queue));
        w(REG_QUEUE_NUM, 16);
        let desc = 0x1000 + 0x4000 * u64::from(queue);
        w(REG_QUEUE_DESC_LOW, desc);
        w(REG_QUEUE_DRIVER_LOW, desc + 0x1000);
        w(REG_QUEUE_DEVICE_LOW, desc + 0x2000);
        w(REG_QUEUE_READY, 1);
    }
    assert_eq!(w(REG_STATUS, 15), TransportEvent::DriverOk);
}

#[test]
fn table_matches_the_device_surface_and_command_line_exactly() {
    assert_eq!(kernel_command_line(DeviceSet::FULL), SPEC_COMMAND_LINE);
    assert_eq!(
        kernel_command_line(DeviceSet::new(false, false)),
        "virtio_mmio.device=4K@0xd0000000:5:0 virtio_mmio.device=4K@0xd0003000:8:3 \
virtio_mmio.device=4K@0xd0004000:9:4"
    );
    let gsis: Vec<u32> = Slot::ALL.iter().map(|slot| slot.gsi()).collect();
    assert_eq!(gsis, vec![5, 6, 7, 8, 9]);
    let ids: Vec<u32> = Slot::ALL.iter().map(|slot| slot.device_id()).collect();
    assert_eq!(ids, vec![2, 2, 1, 19, 4]);
    let queues: Vec<u16> = Slot::ALL.iter().map(|slot| slot.queue_count()).collect();
    assert_eq!(queues, vec![1, 1, 2, 3, 1]);
    assert_eq!(Slot::from_gpa(0xcfff_ffff), None);
    assert_eq!(Slot::from_gpa(0xd000_0000), Some((Slot::Root, 0)));
    assert_eq!(Slot::from_gpa(0xd000_0fff), Some((Slot::Root, 0xfff)));
    assert_eq!(Slot::from_gpa(0xd000_1000), Some((Slot::Overlay, 0)));
    assert_eq!(Slot::from_gpa(0xd000_4fff), Some((Slot::Rng, 0xfff)));
    assert_eq!(Slot::from_gpa(0xd000_5000), None);
    assert_eq!(Slot::from_gpa(u64::MAX), None);
    assert_eq!(Slot::from_gpa(0), None);
    assert_eq!(Slot::Vsock.end(), 0xd000_3fff);
    assert_eq!(Slot::Net.notify_addr(), 0xd000_2050);
}

#[test]
fn dispatch_reaches_each_slot_and_rejects_unmapped_addresses() {
    let mut bus = bus();
    for slot in Slot::ALL {
        assert_eq!(bus.dispatch_read(slot.base(), W), Ok(0x7472_6976));
        assert_eq!(bus.dispatch_read(slot.base() + REG_VERSION, W), Ok(2));
        assert_eq!(
            bus.dispatch_read(slot.base() + REG_DEVICE_ID, W),
            Ok(u64::from(slot.device_id()))
        );
        assert!(!bus.interrupt_pending(slot));
    }
    assert_eq!(
        bus.dispatch_read(0xd000_5000, W),
        Err(BusViolation::UnmappedAddress { gpa: 0xd000_5000 })
    );
    let mem = crate::virtio::guest_memory::VecGuestMemory::flat(0x1000).expect("mem");
    assert_eq!(
        bus.dispatch_write(0xcfff_fffc, W, 1, &mem),
        Err(BusViolation::UnmappedAddress { gpa: 0xcfff_fffc })
    );
    let bad = bus.dispatch_read(Slot::Net.base() + REG_DRIVER_FEATURES, W);
    assert!(matches!(
        bad,
        Err(BusViolation::Transport {
            slot: Slot::Net,
            ..
        })
    ));
    assert_eq!(bus.net().expect("net").violations().total(), 1);
    assert_eq!(
        bus.dispatch_read(Slot::Vsock.base() + 0x100, AccessWidth::U64),
        Ok(CID)
    );
}

#[test]
fn notification_routes_to_the_right_slot_and_interrupt_uses_its_gsi() {
    let mut bus = bus();
    let mut rig = GuestRig::new(&[16]);
    init_slot(&mut bus, &rig, Slot::Rng, RNG_FEATURES);
    let buffer = rig.alloc_zeroed(32);
    rig.submit(0, &[Seg::writable(buffer, 32)]);
    let event = bus
        .dispatch_write(Slot::Rng.notify_addr(), W, 0, &rig.mem)
        .expect("notify");
    assert_eq!(event.notify(), Some((Slot::Rng, 0)));
    let report = bus.service(Slot::Rng, 0, &rig.mem, 8).expect("service");
    assert_eq!((report.completed, report.interrupt), (1, true));
    assert!(bus.interrupt_pending(Slot::Rng));
    assert_eq!(rig.read(buffer, 4), [0, 1, 2, 3]);
    let mut sink = Sink(Vec::new());
    bus.signal(Slot::Rng, &mut sink).expect("signal");
    bus.signal(Slot::Root, &mut sink).expect("signal");
    assert_eq!(sink.0, vec![9, 5]);
    assert!(
        bus.service(Slot::Rng, 1, &rig.mem, 8).is_err(),
        "queue index outside the slot"
    );
    assert!(
        bus.service(Slot::Root, 0, &rig.mem, 8).is_err(),
        "inactive slot"
    );
    assert!(
        bus.dispatch_write(Slot::Root.notify_addr(), W, 0, &rig.mem)
            .is_err()
    );
}

#[test]
fn notify_sources_enumerate_every_slot_queue_pair_once() {
    struct Source(Vec<(u64, Slot, u16)>);
    impl NotifySource for Source {
        type Error = ();
        fn register(&mut self, addr: GuestAddress, slot: Slot, queue: u16) -> Result<(), ()> {
            self.0.push((addr.raw(), slot, queue));
            Ok(())
        }
    }
    let bus = bus();
    let mut source = Source(Vec::new());
    bus.register_notify_sources(&mut source).expect("register");
    assert_eq!(source.0.len(), 8);
    assert_eq!(source.0[0], (0xd000_0050, Slot::Root, 0));
    assert_eq!(source.0[3], (0xd000_2050, Slot::Net, 1));
    assert_eq!(source.0[6], (0xd000_3050, Slot::Vsock, 2));
    assert_eq!(source.0[7], (0xd000_4050, Slot::Rng, 0));
}

#[test]
fn constructor_rejects_block_devices_in_the_wrong_slot() {
    let mut wrong = devices([0; 6]);
    wrong.root = block(BlockRole::PrivateOverlay);
    let error = MmioBus::new(wrong).err();
    assert_eq!(
        error,
        Some(BusConfigError::BlockRole {
            slot: Slot::Root,
            role: BlockRole::PrivateOverlay
        })
    );
}

#[test]
fn snapshot_round_trips_through_restore_and_fails_closed_on_mismatch() {
    let mut bus = bus();
    let rig = GuestRig::new(&[16, 16]);
    init_slot(&mut bus, &rig, Slot::Net, NET_FEATURES);
    let snapshots = bus.snapshot_all();
    assert_eq!(snapshots.len(), 5);
    assert_eq!(snapshots[2].transport.status, 15);
    assert!(
        snapshots[0].transport.queues[0].size == 256,
        "untouched slot stays reset"
    );
    let array: Vec<SlotSnapshot> = snapshots.clone();
    let restored = MmioBus::restore(devices([0; 6]), &array, &rig.mem);
    let mut restored = restored.expect("restore");
    let net = restored.net().expect("net");
    assert!(net.is_active());
    assert_eq!(
        net.device().mac(),
        [2, 0, 0, 0, 0, 1],
        "placeholder MAC restored"
    );
    assert!(!net.device().link_up());
    assert_eq!(
        restored.vsock().device().pending_events(),
        1,
        "TRANSPORT_RESET queued"
    );
    assert_eq!(
        restored.snapshot(Slot::Net).expect("net record").transport,
        snapshots[2].transport
    );

    let mut swapped = array.clone();
    swapped.swap(0, 1);
    let error = MmioBus::restore(devices([0; 6]), &swapped, &rig.mem).err();
    assert_eq!(
        error,
        Some(SlotRestoreError::SlotMismatch {
            expected: Slot::Root,
            actual: Slot::Overlay
        })
    );

    let mut wrong_device = array.clone();
    wrong_device[4].device[1] = 7;
    let error = MmioBus::restore(devices([0; 6]), &wrong_device, &rig.mem).err();
    assert_eq!(
        error,
        Some(SlotRestoreError::Device {
            slot: Slot::Rng,
            error: DeviceStateError::Incompatible
        })
    );
}
