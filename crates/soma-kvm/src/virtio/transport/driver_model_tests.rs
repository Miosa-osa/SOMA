//! A model Linux-style driver walks the full init sequence and exchanges
//! chains with the echo test device, then the whole state is snapshotted and
//! restored.

use super::*;
use crate::virtio::device::test_device::{TEST_FEATURE, TestDevice};
use crate::virtio::guest_memory::{GuestAddress, GuestMemory, VecGuestMemory};
use crate::virtio::queue::chain::{ChainLimits, Descriptor, VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE};
use crate::virtio::queue::violation::{QueueViolation, QueueViolationKind};
use crate::virtio::transport::state::TransportState;
use registers::*;
use status::*;

const W: AccessWidth = AccessWidth::U32;
const SIZE: u16 = 8;
const DESC: u64 = 0x1000;
const AVAIL: u64 = 0x2000;
const USED: u64 = 0x3000;
const OUT: u64 = 0x4000;
const IN: u64 = 0x5000;
const LIMITS: ChainLimits = ChainLimits {
    max_descriptors: 8,
    max_bytes: 4096,
};

pub(super) fn memory() -> VecGuestMemory {
    VecGuestMemory::flat(0x8000).expect("memory")
}

pub(super) fn init_driver(t: &mut MmioTransport<TestDevice>, mem: &VecGuestMemory) {
    let mut w = |offset: u64, value: u64| {
        t.write(offset, W, value, mem)
            .unwrap_or_else(|violation| panic!("write {offset:#x}: {violation}"))
    };
    assert_eq!(w(REG_STATUS, 0), TransportEvent::Reset);
    w(REG_STATUS, u64::from(STATUS_ACKNOWLEDGE));
    w(REG_STATUS, u64::from(STATUS_ACKNOWLEDGE | STATUS_DRIVER));
    w(REG_DRIVER_FEATURES_SEL, 0);
    w(REG_DRIVER_FEATURES, TEST_FEATURE);
    w(REG_DRIVER_FEATURES_SEL, 1);
    w(REG_DRIVER_FEATURES, 1);
    w(REG_STATUS, 11);
    w(REG_QUEUE_SEL, 0);
    w(REG_QUEUE_NUM, u64::from(SIZE));
    w(REG_QUEUE_DESC_LOW, DESC);
    w(REG_QUEUE_DESC_HIGH, 0);
    w(REG_QUEUE_DRIVER_LOW, AVAIL);
    w(REG_QUEUE_DRIVER_HIGH, 0);
    w(REG_QUEUE_DEVICE_LOW, USED);
    w(REG_QUEUE_DEVICE_HIGH, 0);
    w(REG_QUEUE_READY, 1);
    assert_eq!(w(REG_STATUS, 15), TransportEvent::DriverOk);
}

/// The device side: pop every pending chain, echo readable bytes into the
/// writable segment, and complete it; returns whether an interrupt is due.
fn service_queue(t: &mut MmioTransport<TestDevice>, mem: &VecGuestMemory) -> bool {
    let mut interrupt = false;
    loop {
        let (queue, _device) = t.queue_and_device_mut(0).expect("queue 0");
        let chain = match queue.pop_descriptor_chain(mem, LIMITS) {
            Ok(Some(chain)) => chain,
            Ok(None) => break,
            Err(QueueViolation::Chain { .. }) => continue,
            Err(other) => panic!("queue failure {other}"),
        };
        let mut payload = Vec::new();
        for segment in chain.readable() {
            let mut buf = vec![0u8; usize::try_from(segment.len).expect("small")];
            mem.read_bytes(segment.addr, &mut buf).expect("readable");
            payload.extend_from_slice(&buf);
        }
        let sink = chain.writable().next().expect("writable segment");
        let written = payload.len().min(usize::try_from(sink.len).expect("small"));
        mem.write_bytes(sink.addr, &payload[..written])
            .expect("writable");
        let len = u32::try_from(written).expect("small");
        interrupt |= t.complete_used(0, mem, &chain, len).expect("complete");
    }
    interrupt
}

fn submit(mem: &VecGuestMemory, slot: u16, message: &[u8]) {
    let head = slot % SIZE;
    let tail = (head + 1) % SIZE;
    let out = OUT + 0x100 * u64::from(head);
    let into = IN + 0x100 * u64::from(head);
    mem.write_bytes(GuestAddress(out), message)
        .expect("payload");
    let request = Descriptor {
        addr: out,
        len: u32::try_from(message.len()).expect("small"),
        flags: VIRTQ_DESC_F_NEXT,
        next: tail,
    };
    let response = Descriptor {
        addr: into,
        len: 0x100,
        flags: VIRTQ_DESC_F_WRITE,
        next: 0,
    };
    mem.write_bytes(
        GuestAddress(DESC + 16 * u64::from(head)),
        &request.to_bytes(),
    )
    .expect("desc");
    mem.write_bytes(
        GuestAddress(DESC + 16 * u64::from(tail)),
        &response.to_bytes(),
    )
    .expect("desc");
    mem.write_obj_at(GuestAddress(AVAIL + 4 + 2 * u64::from(slot % SIZE)), head)
        .expect("ring");
    mem.write_obj_at(GuestAddress(AVAIL + 2), slot.wrapping_add(1))
        .expect("idx");
}

#[test]
fn driver_model_initializes_and_exchanges_chains_end_to_end() {
    let mem = memory();
    let mut t = MmioTransport::new(TestDevice::default()).expect("transport");
    init_driver(&mut t, &mem);
    assert!(t.is_active());
    assert_eq!(t.device().activated_with, Some((1 << 32) | TEST_FEATURE));

    submit(&mem, 0, b"hello");
    assert_eq!(
        t.write(REG_QUEUE_NOTIFY, W, 0, &mem),
        Ok(TransportEvent::QueueNotify(0))
    );
    assert!(service_queue(&mut t, &mem), "driver wants an interrupt");
    assert_eq!(t.read(REG_INTERRUPT_STATUS, W), Ok(1));
    let mut echoed = [0u8; 5];
    mem.read_bytes(GuestAddress(IN), &mut echoed).expect("echo");
    assert_eq!(&echoed, b"hello");
    assert_eq!(mem.read_obj_at::<u16>(GuestAddress(USED + 2)), Ok(1));
    assert_eq!(mem.read_obj_at::<u32>(GuestAddress(USED + 8)), Ok(5));
    t.write(REG_INTERRUPT_ACK, W, 1, &mem).expect("ack");
    assert_eq!(t.interrupt_status(), 0);

    submit(&mem, 1, b"a");
    submit(&mem, 2, b"bb");
    assert!(service_queue(&mut t, &mem));
    assert_eq!(mem.read_obj_at::<u16>(GuestAddress(USED + 2)), Ok(3));
    assert_eq!(t.queue(0).expect("queue").state().next_avail, 3);
    assert_eq!(t.violations().total(), 0);
}

#[test]
fn hostile_chain_is_skipped_and_counted_while_good_chains_still_flow() {
    let mem = memory();
    let mut t = MmioTransport::new(TestDevice::default()).expect("transport");
    init_driver(&mut t, &mem);
    let bad = Descriptor {
        addr: 0x1_0000,
        len: 4,
        flags: VIRTQ_DESC_F_WRITE,
        next: 0,
    };
    mem.write_bytes(GuestAddress(DESC + 16 * 6), &bad.to_bytes())
        .expect("desc");
    mem.write_obj_at(GuestAddress(AVAIL + 4), 6u16)
        .expect("ring");
    mem.write_obj_at(GuestAddress(AVAIL + 2), 1u16)
        .expect("idx");
    submit(&mem, 1, b"ok");
    assert!(service_queue(&mut t, &mem));
    let mut echoed = [0u8; 2];
    mem.read_bytes(GuestAddress(IN + 0x100), &mut echoed)
        .expect("echo");
    assert_eq!(&echoed, b"ok");
    let queue = t.queue(0).expect("queue");
    assert_eq!(queue.violations().count(QueueViolationKind::Chain), 1);
    assert_eq!(queue.state().next_avail, 2);
}

#[test]
fn transport_state_round_trips_through_bytes_and_restore() {
    let mem = memory();
    let mut t = MmioTransport::new(TestDevice::default()).expect("transport");
    init_driver(&mut t, &mem);
    submit(&mem, 0, b"x");
    service_queue(&mut t, &mem);
    t.signal_config_change();
    t.write(REG_QUEUE_SEL, W, 1, &mem).expect("select");
    let state = t.state();
    assert_eq!(state.status, 15);
    assert_eq!(state.interrupt_status, 3);
    assert_eq!(state.queues[0].next_used, 1);

    let raw = state.to_bytes().expect("encode");
    assert_eq!(raw.len(), state::TRANSPORT_STATE_HEADER_LEN + 2 * 32);
    assert_eq!(TransportState::from_bytes(&raw), Ok(state.clone()));
    assert!(TransportState::from_bytes(&raw[..raw.len() - 1]).is_err());
    let mut bad_count_byte = raw.clone();
    bad_count_byte[29] = 9;
    assert!(TransportState::from_bytes(&bad_count_byte).is_err());

    let device_bytes = t.device().snapshot_state();
    let mut device = TestDevice::default();
    device.restore_state(&device_bytes).expect("device state");
    let mut restored = MmioTransport::restore(device, &state, &mem).expect("restore");
    assert_eq!(restored.state(), state);
    assert!(restored.is_active());
    assert_eq!(
        restored.device().activated_with,
        Some((1 << 32) | TEST_FEATURE)
    );
    assert_eq!(restored.read(REG_QUEUE_NUM_MAX, W), Ok(16));

    submit(&mem, 1, b"after");
    assert!(service_queue(&mut restored, &mem));
    assert_eq!(mem.read_obj_at::<u16>(GuestAddress(USED + 2)), Ok(2));
}
