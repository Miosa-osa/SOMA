//! End-to-end network tests through the transport with a model driver.

use super::backend::{LoopbackBackend, LoopbackHandle};
use super::frame::{MAX_FRAME_LEN, VIRTIO_NET_HDR_LEN};
use super::rx::deliver_rx;
use super::state::{NET_STATE_LEN, NetState};
use super::*;
use crate::virtio::devices::harness::{GuestRig, Seg};
use crate::virtio::devices::service::{ServiceError, service_queue};
use crate::virtio::transport::MmioTransport;
use crate::virtio::transport::registers::{AccessWidth, REG_INTERRUPT_STATUS};

pub(super) const MAC: [u8; 6] = [0x02, 0x53, 0x4f, 0x4d, 0x41, 0x01];
const HDR: u32 = 12;

pub(super) fn boot(
    echo: bool,
    link_up: bool,
) -> (GuestRig, MmioTransport<NetDevice>, LoopbackHandle) {
    let backend = LoopbackBackend::default().with_echo(echo);
    let handle = backend.handle();
    let mut device = NetDevice::new(Box::new(backend), MAC);
    device.set_link(link_up);
    let rig = GuestRig::new(&[32, 32]);
    let mut t = MmioTransport::new(device).expect("transport");
    rig.init(&mut t, NET_FEATURES);
    (rig, t, handle)
}

pub(super) fn frame(len: usize, fill: u8) -> Vec<u8> {
    let mut frame = vec![fill; len];
    frame[..6].copy_from_slice(&[0xff; 6]);
    frame
}

/// Submits `header ++ frame` as one readable chain and services the TX queue.
pub(super) fn transmit(
    rig: &mut GuestRig,
    t: &mut MmioTransport<NetDevice>,
    header: &[u8; VIRTIO_NET_HDR_LEN],
    frame: &[u8],
) -> Result<bool, ServiceError> {
    let hdr = rig.alloc(header);
    let mut segments = vec![Seg::readable(hdr, HDR)];
    if !frame.is_empty() {
        let data = rig.alloc(frame);
        segments.push(Seg::readable(
            data,
            u32::try_from(frame.len()).expect("small"),
        ));
    }
    rig.submit(NET_TX_QUEUE, &segments);
    rig.notify(t, NET_TX_QUEUE);
    let report = service_queue(t, &rig.mem, NET_TX_QUEUE, 8)?;
    assert_eq!(report.completed + report.rejected, 1);
    Ok(report.interrupt)
}

/// Posts one writable receive buffer of `len` bytes and returns its address.
pub(super) fn post_rx(rig: &mut GuestRig, len: u32) -> u64 {
    let addr = rig.alloc_zeroed(len);
    rig.submit(NET_RX_QUEUE, &[Seg::writable(addr, len)]);
    addr
}

fn sent(t: &MmioTransport<NetDevice>) -> usize {
    t.device().counters().tx_ok as usize
}

#[test]
fn transmit_reaches_backend_only_while_link_is_up() {
    let (mut rig, mut t, host) = boot(false, false);
    let frame = frame(60, 0x11);
    assert_eq!(transmit(&mut rig, &mut t, &[0; 12], &frame), Ok(true));
    assert_eq!(t.device().counters().tx_dropped, 1);
    assert_eq!(rig.used_elem(NET_TX_QUEUE, 0).1, 0);
    t.device_mut().set_link(true);
    assert_eq!(transmit(&mut rig, &mut t, &[0; 12], &frame), Ok(true));
    assert_eq!(sent(&t), 1);
    assert_eq!(t.read(REG_INTERRUPT_STATUS, AccessWidth::U32), Ok(1));
    let max = self::frame(MAX_FRAME_LEN, 0x22);
    assert_eq!(transmit(&mut rig, &mut t, &[0; 12], &max), Ok(true));
    assert_eq!(sent(&t), 2);
    assert_eq!(host.take_sent(), vec![frame, max]);
}

#[test]
fn transmit_rejects_nonzero_headers_bad_lengths_and_writable_segments() {
    let (mut rig, mut t, _) = boot(false, true);
    let good = frame(64, 0x33);
    let mut flags = [0u8; 12];
    flags[0] = 1;
    let mut gso = [0u8; 12];
    gso[1] = 1;
    let mut num_buffers = [0u8; 12];
    num_buffers[10] = 1;
    for header in [flags, gso, num_buffers] {
        transmit(&mut rig, &mut t, &header, &good).expect("service");
    }
    transmit(&mut rig, &mut t, &[0; 12], &frame(13, 0)).expect("service");
    transmit(&mut rig, &mut t, &[0; 12], &frame(MAX_FRAME_LEN + 1, 0)).expect("service");
    transmit(&mut rig, &mut t, &[0; 12], &[]).expect("service");
    let hdr = rig.alloc(&[0; 12]);
    let data = rig.alloc(&good);
    let sink = rig.alloc_zeroed(16);
    rig.submit(
        NET_TX_QUEUE,
        &[
            Seg::readable(hdr, HDR),
            Seg::readable(data, 64),
            Seg::writable(sink, 16),
        ],
    );
    let report = service_queue(&mut t, &rig.mem, NET_TX_QUEUE, 8).expect("service");
    assert_eq!(report.completed, 1);
    let counters = t.device().counters();
    assert_eq!((counters.tx_ok, counters.tx_dropped), (0, 6));
    assert_eq!(
        t.queue(NET_TX_QUEUE).expect("queue").violations().total(),
        1,
        "the 1515-byte frame is rejected by the chain walker"
    );
    assert!(t.is_active());
}

#[test]
fn receive_delivers_header_and_frame_into_a_posted_buffer() {
    let (mut rig, mut t, host) = boot(false, true);
    let empty = deliver_rx(&mut t, &rig.mem, 8).expect("deliver");
    assert_eq!(empty.completed, 0, "no buffer posted, nothing read");
    let buffer = post_rx(&mut rig, 2048);
    let idle = deliver_rx(&mut t, &rig.mem, 8).expect("deliver");
    assert_eq!(idle.completed, 0, "backend idle, chain untouched");
    assert_eq!(rig.used_idx(NET_RX_QUEUE), 0);
    let frame = frame(100, 0x44);
    host.push_inbound(&frame);
    let report = deliver_rx(&mut t, &rig.mem, 8).expect("deliver");
    assert_eq!((report.completed, report.interrupt), (1, true));
    assert_eq!(rig.used_elem(NET_RX_QUEUE, 0).1, 112);
    assert_eq!(rig.read(buffer, 12), [0u8; 12]);
    assert_eq!(rig.read(buffer + 12, 100), frame);
    assert_eq!(t.device().counters().rx_ok, 1);
}

#[test]
fn receive_drops_oversized_frames_small_buffers_and_readable_chains() {
    let (mut rig, mut t, host) = boot(false, true);
    post_rx(&mut rig, 2048);
    host.push_inbound(&frame(MAX_FRAME_LEN + 1, 0));
    let report = deliver_rx(&mut t, &rig.mem, 8).expect("deliver");
    assert_eq!(
        report.completed, 0,
        "oversized frame consumed without a chain"
    );
    assert_eq!(t.device().counters().rx_dropped, 1);
    host.push_inbound(&frame(13, 0));
    deliver_rx(&mut t, &rig.mem, 8).expect("deliver");
    assert_eq!(t.device().counters().rx_dropped, 2);
    host.push_inbound(&frame(1000, 0x55));
    deliver_rx(&mut t, &rig.mem, 8).expect("deliver");
    assert_eq!(rig.used_elem(NET_RX_QUEUE, 0).1, 1012);

    post_rx(&mut rig, 64);
    host.push_inbound(&frame(200, 0x66));
    let report = deliver_rx(&mut t, &rig.mem, 8).expect("deliver");
    assert_eq!(report.completed, 1);
    assert_eq!(
        rig.used_elem(NET_RX_QUEUE, 1).1,
        0,
        "small buffer returned empty"
    );
    let readable = rig.alloc_zeroed(2048);
    rig.submit(NET_RX_QUEUE, &[Seg::readable(readable, 2048)]);
    host.push_inbound(&frame(200, 0x77));
    deliver_rx(&mut t, &rig.mem, 8).expect("deliver");
    assert_eq!(rig.used_elem(NET_RX_QUEUE, 2).1, 0);
    assert_eq!(t.device().counters().rx_dropped, 4);
    assert!(t.is_active());
}

#[test]
fn loopback_echoes_a_transmitted_frame_back_to_the_guest() {
    let (mut rig, mut t, _) = boot(true, true);
    let buffer = post_rx(&mut rig, 2048);
    let frame = frame(300, 0x88);
    transmit(&mut rig, &mut t, &[0; 12], &frame).expect("tx");
    let report = deliver_rx(&mut t, &rig.mem, 8).expect("deliver");
    assert_eq!(report.completed, 1);
    assert_eq!(rig.read(buffer + 12, 300), frame);
    t.device_mut().set_link(false);
    post_rx(&mut rig, 2048);
    let down = deliver_rx(&mut t, &rig.mem, 8).expect("deliver");
    assert_eq!(down.completed, 0, "link down stops delivery");
}

#[test]
fn backend_failure_stops_the_device_with_needs_reset() {
    let backend = LoopbackBackend::default().with_failure(std::io::ErrorKind::BrokenPipe);
    let mut device = NetDevice::new(Box::new(backend), MAC);
    device.set_link(true);
    let mut rig = GuestRig::new(&[32, 32]);
    let mut t = MmioTransport::new(device).expect("transport");
    rig.init(&mut t, NET_FEATURES);
    assert_eq!(
        transmit(&mut rig, &mut t, &[0; 12], &frame(64, 1)),
        Ok(true)
    );
    assert_eq!(
        t.device().counters().tx_dropped,
        1,
        "transmit failure is a drop"
    );
    post_rx(&mut rig, 2048);
    assert_eq!(
        deliver_rx(&mut t, &rig.mem, 8),
        Err(ServiceError::Fault(DeviceFault::Backend))
    );
    assert!(!t.is_active());
}

#[test]
fn config_exposes_mac_read_only_and_set_mac_updates_it() {
    let (_, mut t, _) = boot(false, false);
    let mut mac = [0u8; 6];
    for (index, byte) in mac.iter_mut().enumerate() {
        *byte = u8::try_from(t.read(0x100 + index as u64, AccessWidth::U8).expect("cfg"))
            .expect("byte");
    }
    assert_eq!(mac, MAC);
    let mem = crate::virtio::guest_memory::VecGuestMemory::flat(16).expect("mem");
    assert!(t.write(0x100, AccessWidth::U8, 1, &mem).is_err());
    t.device_mut().set_mac([1, 2, 3, 4, 5, 6]);
    t.signal_config_change();
    assert_eq!(t.read(0x100, AccessWidth::U32,), Ok(0x0403_0201));
    assert_eq!(t.read(REG_INTERRUPT_STATUS, AccessWidth::U32), Ok(2));
}

#[test]
fn snapshot_state_round_trips_and_refuses_link_up_or_mismatch() {
    let (_, mut t, _) = boot(false, false);
    let raw = t.device().snapshot_state();
    assert_eq!(raw.len(), NET_STATE_LEN);
    let state = NetState::from_bytes(&raw).expect("decode");
    assert_eq!(state.to_bytes().to_vec(), raw);
    let mut fresh = NetDevice::new(Box::new(LoopbackBackend::default()), [0; 6]);
    assert_eq!(fresh.restore_state(&raw), Ok(()));
    assert_eq!(fresh.mac(), MAC);
    assert!(!fresh.link_up());
    t.device_mut().set_link(true);
    let up = t.device().snapshot_state();
    assert_eq!(
        fresh.restore_state(&up),
        Err(DeviceStateError::Incompatible)
    );
    let mut bad_flag = raw.clone();
    bad_flag[19] = 2;
    assert_eq!(
        fresh.restore_state(&bad_flag),
        Err(DeviceStateError::Malformed)
    );
    let mut wrong_id = raw.clone();
    wrong_id[1] = 2;
    assert_eq!(
        fresh.restore_state(&wrong_id),
        Err(DeviceStateError::Incompatible)
    );
    let mut wrong_features = raw.clone();
    wrong_features[5] ^= 1;
    assert_eq!(
        fresh.restore_state(&wrong_features),
        Err(DeviceStateError::Incompatible)
    );
    assert_eq!(
        fresh.restore_state(&raw[1..]),
        Err(DeviceStateError::Malformed)
    );
}
