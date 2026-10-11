//! The memory device through the transport with a model driver: the requests a real guest sends,
//! and the ones a hostile one would.

use std::sync::{Arc, Mutex, atomic::Ordering};

use super::backing::RecordingBacking;
use super::*;
use crate::virtio::devices::harness::{GuestRig, Seg};
use crate::virtio::devices::service::service_queue;
use crate::virtio::transport::MmioTransport;
use crate::virtio::transport::registers::{AccessWidth, REG_INTERRUPT_STATUS};

pub(super) const REGION: u64 = 0x1_0000_0000;
pub(super) const BLOCKS: u64 = 64;

pub(super) struct Rig {
    pub(super) guest: GuestRig,
    pub(super) t: MmioTransport<MemDevice>,
    pub(super) plugged: Arc<Mutex<Vec<(u64, u64)>>>,
    pub(super) discarded: Arc<Mutex<Vec<(u64, u64)>>>,
    pub(super) fail: Arc<std::sync::atomic::AtomicBool>,
}

pub(super) fn boot() -> Rig {
    let backing = RecordingBacking::default();
    let plugged = Arc::clone(&backing.plugged);
    let discarded = Arc::clone(&backing.discarded);
    let fail = Arc::clone(&backing.fail);
    let device = MemDevice::new(REGION, BLOCKS * BLOCK_SIZE, Box::new(backing)).expect("device");
    let rig = GuestRig::new(&[16]);
    let mut t = MmioTransport::new(device).expect("transport");
    rig.init(&mut t, MEM_FEATURES);
    Rig {
        guest: rig,
        t,
        plugged,
        discarded,
        fail,
    }
}

pub(super) fn request(kind: u16, addr: u64, blocks: u16) -> [u8; REQUEST_LEN] {
    let mut raw = [0_u8; REQUEST_LEN];
    raw[0..2].copy_from_slice(&kind.to_le_bytes());
    raw[8..16].copy_from_slice(&addr.to_le_bytes());
    raw[16..18].copy_from_slice(&blocks.to_le_bytes());
    raw
}

/// Sends one request and returns `(response type, state)`.
pub(super) fn ask(r: &mut Rig, kind: u16, addr: u64, blocks: u16) -> (u16, u16) {
    let body = r.guest.alloc(&request(kind, addr, blocks));
    let reply = r.guest.alloc_zeroed(RESPONSE_WIRE);
    r.guest.submit(
        0,
        &[
            Seg::readable(body, REQUEST_WIRE),
            Seg::writable(reply, RESPONSE_WIRE),
        ],
    );
    r.guest.notify(&mut r.t, 0);
    let report = service_queue(&mut r.t, &r.guest.mem, 0, 8).expect("service");
    assert_eq!(report.completed, 1);
    let raw = r.guest.read(reply, RESPONSE_LEN);
    (
        u16::from_le_bytes([raw[0], raw[1]]),
        u16::from_le_bytes([raw[8], raw[9]]),
    )
}

pub(super) fn block(index: u64) -> u64 {
    REGION + index * BLOCK_SIZE
}

pub(super) fn request_size(r: &mut Rig, bytes: u64) {
    r.t.device_mut().set_requested_size(bytes).expect("size");
    r.t.signal_config_change();
}

#[test]
fn the_config_space_describes_an_empty_region_and_follows_the_request() {
    let mut r = boot();
    let read = |r: &Rig, offset: usize| {
        let mut raw = [0_u8; 8];
        r.t.device().read_config(offset, &mut raw).expect("config");
        u64::from_le_bytes(raw)
    };
    assert_eq!(read(&r, 0), BLOCK_SIZE);
    assert_eq!(read(&r, 16), REGION);
    assert_eq!(read(&r, 24), BLOCKS * BLOCK_SIZE);
    assert_eq!(
        read(&r, 40),
        0,
        "boot memory is all there is: nothing plugged"
    );
    assert_eq!(read(&r, 48), 0);
    request_size(&mut r, 8 * BLOCK_SIZE);
    assert_eq!(read(&r, 48), 8 * BLOCK_SIZE);
    assert_eq!(
        r.t.read(REG_INTERRUPT_STATUS, AccessWidth::U32),
        Ok(u64::from(crate::virtio::transport::INTERRUPT_CONFIG_CHANGE))
    );
    assert!(r.t.device().read_config(50, &mut [0_u8; 8]).is_err());
}

#[test]
fn a_guest_gets_exactly_what_was_asked_for_and_no_more() {
    let mut r = boot();
    request_size(&mut r, 4 * BLOCK_SIZE);
    assert_eq!(ask(&mut r, REQ_PLUG, block(0), 3), (RESP_ACK, 0));
    assert_eq!(r.t.device().plugged_size(), 3 * BLOCK_SIZE);
    // One more block fits, two do not.
    assert_eq!(ask(&mut r, REQ_PLUG, block(3), 2), (RESP_NACK, 0));
    assert_eq!(r.t.device().plugged_size(), 3 * BLOCK_SIZE);
    assert_eq!(ask(&mut r, REQ_PLUG, block(3), 1), (RESP_ACK, 0));
    assert_eq!(r.t.device().plugged_size(), 4 * BLOCK_SIZE);
    assert_eq!(r.t.device().counters().nacked, 1);
}

#[test]
fn an_unplug_returns_the_memory_to_the_host() {
    let mut r = boot();
    request_size(&mut r, 4 * BLOCK_SIZE);
    assert_eq!(ask(&mut r, REQ_PLUG, block(0), 4), (RESP_ACK, 0));
    assert_eq!(ask(&mut r, REQ_UNPLUG, block(1), 2), (RESP_ACK, 0));
    assert_eq!(r.t.device().plugged_size(), 2 * BLOCK_SIZE);
    assert_eq!(
        *r.discarded.lock().expect("log"),
        vec![(BLOCK_SIZE, 2 * BLOCK_SIZE)]
    );
    // The same blocks cannot be unplugged twice.
    assert_eq!(ask(&mut r, REQ_UNPLUG, block(1), 1).0, RESP_ERROR);
    assert_eq!(r.t.device().plugged_size(), 2 * BLOCK_SIZE);
}

#[test]
fn a_block_the_host_cannot_take_back_stays_plugged() {
    let mut r = boot();
    request_size(&mut r, 2 * BLOCK_SIZE);
    assert_eq!(ask(&mut r, REQ_PLUG, block(0), 2), (RESP_ACK, 0));
    r.fail.store(true, Ordering::Relaxed);
    assert_eq!(ask(&mut r, REQ_UNPLUG, block(0), 1).0, RESP_ERROR);
    assert_eq!(
        r.t.device().plugged_size(),
        2 * BLOCK_SIZE,
        "no memory is claimed freed"
    );
}

#[test]
fn state_reports_plugged_unplugged_or_mixed() {
    let mut r = boot();
    request_size(&mut r, 2 * BLOCK_SIZE);
    assert_eq!(ask(&mut r, REQ_PLUG, block(0), 1), (RESP_ACK, 0));
    assert_eq!(
        ask(&mut r, REQ_STATE, block(0), 1),
        (RESP_ACK, STATE_PLUGGED)
    );
    assert_eq!(
        ask(&mut r, REQ_STATE, block(1), 1),
        (RESP_ACK, STATE_UNPLUGGED)
    );
    assert_eq!(ask(&mut r, REQ_STATE, block(0), 2), (RESP_ACK, STATE_MIXED));
}

#[test]
fn unplug_all_empties_the_region_and_a_reset_forgets_every_block() {
    let mut r = boot();
    request_size(&mut r, 3 * BLOCK_SIZE);
    assert_eq!(ask(&mut r, REQ_PLUG, block(5), 3), (RESP_ACK, 0));
    assert_eq!(ask(&mut r, REQ_UNPLUG_ALL, 0, 0), (RESP_ACK, 0));
    assert_eq!(r.t.device().plugged_size(), 0);
    assert_eq!(ask(&mut r, REQ_PLUG, block(0), 1), (RESP_ACK, 0));
    r.t.device_mut().reset();
    assert_eq!(r.t.device().plugged_size(), 0);
    assert!(r.t.device().is_quiescent());
}

#[test]
fn the_host_cannot_ask_for_more_than_the_region_or_a_part_of_a_block() {
    let mut r = boot();
    let device = r.t.device_mut();
    assert_eq!(
        device.set_requested_size(BLOCK_SIZE + 1),
        Err(SizeError::Unaligned)
    );
    assert_eq!(
        device.set_requested_size((BLOCKS + 1) * BLOCK_SIZE),
        Err(SizeError::BeyondRegion)
    );
    assert_eq!(device.set_requested_size(BLOCKS * BLOCK_SIZE), Ok(()));
    assert!(
        MemDevice::new(
            REGION + 1,
            BLOCK_SIZE,
            Box::new(RecordingBacking::default())
        )
        .is_err()
    );
    assert!(MemDevice::new(REGION, 0, Box::new(RecordingBacking::default())).is_err());
}

#[test]
fn a_snapshot_records_geometry_only_and_restores_empty() {
    let mut r = boot();
    request_size(&mut r, 2 * BLOCK_SIZE);
    assert_eq!(ask(&mut r, REQ_PLUG, block(0), 2), (RESP_ACK, 0));
    assert!(
        !r.t.device().is_quiescent(),
        "a capture is refused while memory is plugged"
    );
    let record = r.t.device().snapshot_state();
    assert_eq!(record.len(), state::MEM_STATE_LEN);
    r.t.device_mut().restore_state(&record).expect("restore");
    assert!(r.t.device().is_quiescent());
    assert_eq!(r.t.device().requested_size(), 0);
    // A record for another region is refused, as is a damaged one.
    let other = MemDevice::new(
        REGION * 2,
        BLOCKS * BLOCK_SIZE,
        Box::new(RecordingBacking::default()),
    )
    .expect("other");
    assert_eq!(
        r.t.device_mut().restore_state(&other.snapshot_state()),
        Err(DeviceStateError::Incompatible)
    );
    assert_eq!(
        r.t.device_mut().restore_state(&record[1..]),
        Err(DeviceStateError::Malformed)
    );
}

#[test]
fn a_block_the_host_cannot_back_is_refused_and_never_promised() {
    let mut r = boot();
    request_size(&mut r, 2 * BLOCK_SIZE);
    r.fail.store(true, Ordering::Relaxed);
    assert_eq!(ask(&mut r, REQ_PLUG, block(0), 2).0, RESP_ERROR);
    assert_eq!(r.t.device().plugged_size(), 0);
    r.fail.store(false, Ordering::Relaxed);
    assert_eq!(ask(&mut r, REQ_PLUG, block(0), 2), (RESP_ACK, 0));
    assert_eq!(*r.plugged.lock().expect("log"), vec![(0, 2 * BLOCK_SIZE)]);
}
