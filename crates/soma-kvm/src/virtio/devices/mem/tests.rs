//! Memory device tests through the transport with a model driver.

use std::sync::{Arc, Mutex};

use super::backend::{MemBackend, MemBackendError, NullBackend};
use super::bitmap::{BitmapError, MAX_BLOCKS, PluggedBitmap, RangeState};
use super::request::{
    MemRequest, REQUEST_LEN, RESPONSE_LEN, RequestKind, VIRTIO_MEM_RESP_ACK, VIRTIO_MEM_RESP_BUSY,
    VIRTIO_MEM_RESP_ERROR, VIRTIO_MEM_RESP_NACK, VIRTIO_MEM_STATE_MIXED, VIRTIO_MEM_STATE_PLUGGED,
    VIRTIO_MEM_STATE_UNPLUGGED,
};
use super::state::{MEM_STATE_HEADER_LEN, MEM_STATE_VERSION, MemState};
use super::*;
use crate::virtio::devices::harness::{GuestRig, Seg};
use crate::virtio::devices::service::service_queue;
use crate::virtio::transport::MmioTransport;

const MIB: u64 = 1 << 20;
const BLOCK: u64 = 2 * MIB;
const BASE: u64 = 1 << 32;
const BLOCKS: u64 = 64;

type Log = Arc<Mutex<Vec<(&'static str, u64, u64)>>>;

#[derive(Clone, Default)]
struct Recorder {
    log: Log,
    refuse: Arc<Mutex<Option<MemBackendError>>>,
}

impl Recorder {
    fn note(&self, op: &'static str, addr: u64, len: u64) -> Result<(), MemBackendError> {
        if let Some(error) = *self.refuse.lock().expect("lock") {
            return Err(error);
        }
        self.log.lock().expect("lock").push((op, addr, len));
        Ok(())
    }
}

impl MemBackend for Recorder {
    fn plug(&mut self, addr: u64, len: u64) -> Result<(), MemBackendError> {
        self.note("plug", addr, len)
    }

    fn unplug(&mut self, addr: u64, len: u64) -> Result<(), MemBackendError> {
        self.note("unplug", addr, len)
    }

    fn unplug_all(&mut self) -> Result<(), MemBackendError> {
        self.note("unplug_all", 0, 0)
    }
}

fn geometry() -> MemGeometry {
    MemGeometry::new(BLOCK, BASE, BLOCK * BLOCKS, 0).expect("geometry")
}

fn boot_with(backend: Box<dyn MemBackend + Send>) -> (GuestRig, MmioTransport<MemDevice>) {
    let rig = GuestRig::new(&[16]);
    let mut t = MmioTransport::new(MemDevice::new(geometry(), backend)).expect("transport");
    rig.init(&mut t, MEM_FEATURES);
    (rig, t)
}

fn boot() -> (GuestRig, MmioTransport<MemDevice>, Recorder) {
    let recorder = Recorder::default();
    let (rig, t) = boot_with(Box::new(recorder.clone()));
    (rig, t, recorder)
}

fn send(
    rig: &mut GuestRig,
    t: &mut MmioTransport<MemDevice>,
    raw: &[u8],
    writable: u32,
) -> (u16, u16, u32) {
    let request = rig.alloc(raw);
    let response = rig.alloc_zeroed(writable.max(1));
    let readable = u32::try_from(raw.len()).expect("small");
    rig.submit(
        0,
        &[
            Seg::readable(request, readable),
            Seg::writable(response, writable),
        ],
    );
    let before = rig.used_idx(0);
    rig.notify(t, 0);
    service_queue(t, &rig.mem, 0, 8).expect("service");
    assert_eq!(rig.used_idx(0), before.wrapping_add(1));
    let used = rig.used_elem(0, before % 16).1;
    let bytes = rig.read(response, RESPONSE_LEN);
    (
        u16::from_le_bytes([bytes[0], bytes[1]]),
        u16::from_le_bytes([bytes[8], bytes[9]]),
        used,
    )
}

fn ask(
    rig: &mut GuestRig,
    t: &mut MmioTransport<MemDevice>,
    kind: RequestKind,
    block: u64,
    nb_blocks: u16,
) -> (u16, u16) {
    let request = MemRequest {
        kind,
        addr: BASE + block * BLOCK,
        nb_blocks,
    };
    let (kind, state, used) = send(
        rig,
        t,
        &request.to_bytes(),
        u32::try_from(RESPONSE_LEN).expect("small"),
    );
    assert_eq!(used, u32::try_from(RESPONSE_LEN).expect("small"));
    (kind, state)
}

fn request(t: &mut MmioTransport<MemDevice>, blocks: u64) {
    t.device_mut()
        .set_requested_size(blocks * BLOCK)
        .expect("requested size");
}

fn config(t: &MmioTransport<MemDevice>) -> [u8; MEM_CONFIG_LEN] {
    let mut raw = [0u8; MEM_CONFIG_LEN];
    t.device().read_config(0, &mut raw).expect("config");
    raw
}

fn word(raw: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(raw[at..at + 8].try_into().expect("eight"))
}

#[test]
fn config_space_reports_geometry_and_live_sizes_read_only() {
    let (mut rig, mut t, _) = boot();
    let raw = config(&t);
    assert_eq!(word(&raw, 0), BLOCK);
    assert_eq!(word(&raw, 16), BASE);
    assert_eq!(word(&raw, 24), BLOCK * BLOCKS);
    assert_eq!(word(&raw, 32), BLOCK * BLOCKS);
    assert_eq!((word(&raw, 40), word(&raw, 48)), (0, 0));
    request(&mut t, 4);
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Plug, 0, 3).0,
        VIRTIO_MEM_RESP_ACK
    );
    let raw = config(&t);
    assert_eq!((word(&raw, 40), word(&raw, 48)), (3 * BLOCK, 4 * BLOCK));
    let device = t.device_mut();
    assert_eq!(
        device.write_config(0, &[1]),
        Err(ConfigAccessError::ReadOnly)
    );
    assert_eq!(
        device.write_config(48, &[0; 8]),
        Err(ConfigAccessError::ReadOnly)
    );
    assert_eq!(
        device.write_config(55, &[0; 2]),
        Err(ConfigAccessError::OutOfBounds)
    );
    let mut one = [0u8; 1];
    assert_eq!(
        device.read_config(56, &mut one),
        Err(ConfigAccessError::OutOfBounds)
    );
    assert_eq!(
        device.read_config(usize::MAX, &mut one),
        Err(ConfigAccessError::OutOfBounds)
    );
}

#[test]
fn plug_up_to_the_requested_size_then_nack_and_calls_the_backend() {
    let (mut rig, mut t, recorder) = boot();
    request(&mut t, 4);
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Plug, 0, 2).0,
        VIRTIO_MEM_RESP_ACK
    );
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Plug, 2, 2).0,
        VIRTIO_MEM_RESP_ACK
    );
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Plug, 4, 1).0,
        VIRTIO_MEM_RESP_NACK
    );
    assert_eq!(t.device().plugged_size(), 4 * BLOCK);
    assert_eq!(
        *recorder.log.lock().expect("lock"),
        vec![
            ("plug", BASE, 2 * BLOCK),
            ("plug", BASE + 2 * BLOCK, 2 * BLOCK)
        ]
    );
    assert_eq!(t.device().counters().nack, 1);
}

#[test]
fn invalid_ranges_are_errors_and_change_nothing() {
    let (mut rig, mut t, recorder) = boot();
    request(&mut t, 8);
    let bad = [
        MemRequest {
            kind: RequestKind::Plug,
            addr: BASE + 1,
            nb_blocks: 1,
        },
        MemRequest {
            kind: RequestKind::Plug,
            addr: BASE - BLOCK,
            nb_blocks: 1,
        },
        MemRequest {
            kind: RequestKind::Plug,
            addr: BASE,
            nb_blocks: 0,
        },
        MemRequest {
            kind: RequestKind::Plug,
            addr: BASE + (BLOCKS - 1) * BLOCK,
            nb_blocks: 2,
        },
        MemRequest {
            kind: RequestKind::Plug,
            addr: BASE,
            nb_blocks: MAX_REQUEST_BLOCKS + 1,
        },
        MemRequest {
            kind: RequestKind::Plug,
            addr: u64::MAX - BLOCK + 1,
            nb_blocks: u16::MAX,
        },
        MemRequest {
            kind: RequestKind::Unplug,
            addr: BASE,
            nb_blocks: 1,
        },
        MemRequest {
            kind: RequestKind::State,
            addr: BASE + BLOCKS * BLOCK,
            nb_blocks: 1,
        },
        MemRequest {
            kind: RequestKind::State,
            addr: BASE,
            nb_blocks: 0,
        },
    ];
    for request in bad {
        let (kind, _, _) = send(&mut rig, &mut t, &request.to_bytes(), 10);
        assert_eq!(kind, VIRTIO_MEM_RESP_ERROR, "{request:?}");
    }
    assert_eq!(t.device().plugged_size(), 0);
    assert!(recorder.log.lock().expect("lock").is_empty());
}

#[test]
fn double_plug_and_unplug_of_unplugged_blocks_are_errors() {
    let (mut rig, mut t, _) = boot();
    request(&mut t, 8);
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Plug, 2, 2).0,
        VIRTIO_MEM_RESP_ACK
    );
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Plug, 3, 2).0,
        VIRTIO_MEM_RESP_ERROR
    );
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Unplug, 1, 2).0,
        VIRTIO_MEM_RESP_ERROR
    );
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Unplug, 2, 3).0,
        VIRTIO_MEM_RESP_ERROR
    );
    assert_eq!(t.device().plugged_size(), 2 * BLOCK);
}

#[test]
fn state_reports_plugged_unplugged_and_mixed() {
    let (mut rig, mut t, _) = boot();
    request(&mut t, 8);
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Plug, 2, 2).0,
        VIRTIO_MEM_RESP_ACK
    );
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::State, 2, 2),
        (VIRTIO_MEM_RESP_ACK, VIRTIO_MEM_STATE_PLUGGED)
    );
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::State, 0, 2),
        (VIRTIO_MEM_RESP_ACK, VIRTIO_MEM_STATE_UNPLUGGED)
    );
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::State, 1, 3),
        (VIRTIO_MEM_RESP_ACK, VIRTIO_MEM_STATE_MIXED)
    );
}

#[test]
fn unplug_frees_requested_headroom_and_unplug_all_clears() {
    let (mut rig, mut t, recorder) = boot();
    request(&mut t, 4);
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Plug, 0, 4).0,
        VIRTIO_MEM_RESP_ACK
    );
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Unplug, 1, 2).0,
        VIRTIO_MEM_RESP_ACK
    );
    assert_eq!(t.device().plugged_size(), 2 * BLOCK);
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Plug, 1, 2).0,
        VIRTIO_MEM_RESP_ACK
    );
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::UnplugAll, 0, 0).0,
        VIRTIO_MEM_RESP_ACK
    );
    assert_eq!(t.device().plugged_size(), 0);
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::UnplugAll, 0, 0).0,
        VIRTIO_MEM_RESP_ACK
    );
    let log = recorder.log.lock().expect("lock");
    assert_eq!(log[1], ("unplug", BASE + BLOCK, 2 * BLOCK));
    assert_eq!(log[3].0, "unplug_all");
}

#[test]
fn lowering_the_request_does_not_unplug_by_itself() {
    let (mut rig, mut t, _) = boot();
    request(&mut t, 4);
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Plug, 0, 4).0,
        VIRTIO_MEM_RESP_ACK
    );
    request(&mut t, 1);
    assert_eq!(t.device().plugged_size(), 4 * BLOCK);
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Plug, 4, 1).0,
        VIRTIO_MEM_RESP_NACK
    );
}

#[test]
fn backend_busy_and_failure_leave_the_bitmap_unchanged() {
    let (mut rig, mut t, recorder) = boot();
    request(&mut t, 8);
    *recorder.refuse.lock().expect("lock") = Some(MemBackendError::Busy);
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Plug, 0, 2).0,
        VIRTIO_MEM_RESP_BUSY
    );
    *recorder.refuse.lock().expect("lock") = Some(MemBackendError::Failed);
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Plug, 0, 2).0,
        VIRTIO_MEM_RESP_ERROR
    );
    assert_eq!(t.device().plugged_size(), 0);
    *recorder.refuse.lock().expect("lock") = None;
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Plug, 0, 2).0,
        VIRTIO_MEM_RESP_ACK
    );
    *recorder.refuse.lock().expect("lock") = Some(MemBackendError::Busy);
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Unplug, 0, 2).0,
        VIRTIO_MEM_RESP_BUSY
    );
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::UnplugAll, 0, 0).0,
        VIRTIO_MEM_RESP_BUSY
    );
    assert_eq!(t.device().plugged_size(), 2 * BLOCK);
    let counters = t.device().counters();
    assert_eq!((counters.busy, counters.error), (3, 1));
}

#[test]
fn malformed_chains_are_answered_or_counted_never_executed() {
    let (mut rig, mut t, recorder) = boot();
    request(&mut t, 8);
    let good = MemRequest {
        kind: RequestKind::Plug,
        addr: BASE,
        nb_blocks: 1,
    }
    .to_bytes();
    let (kind, _, _) = send(&mut rig, &mut t, &good[..REQUEST_LEN - 1], 10);
    assert_eq!(kind, VIRTIO_MEM_RESP_ERROR, "short request");
    let mut long = good.to_vec();
    long.push(0);
    let (kind, _, _) = send(&mut rig, &mut t, &long, 10);
    assert_eq!(kind, VIRTIO_MEM_RESP_ERROR, "long request");
    let mut unknown = good;
    unknown[0] = 9;
    let (kind, _, _) = send(&mut rig, &mut t, &unknown, 10);
    assert_eq!(kind, VIRTIO_MEM_RESP_ERROR, "unknown type");
    let (_, _, used) = send(&mut rig, &mut t, &good, 9);
    assert_eq!(used, 0, "no room for a response");
    assert_eq!(t.device().plugged_size(), 0);
    assert!(recorder.log.lock().expect("lock").is_empty());
    assert_eq!(t.device().counters().rejected, 3);
}

#[test]
fn requested_size_must_be_whole_blocks_within_the_region() {
    let mut device = MemDevice::new(geometry(), Box::new(NullBackend));
    assert_eq!(device.set_requested_size(BLOCK), Ok(true));
    assert_eq!(device.set_requested_size(BLOCK), Ok(false));
    assert_eq!(
        device.set_requested_size(BLOCK + 1),
        Err(RequestedSizeError::Unaligned)
    );
    assert_eq!(
        device.set_requested_size((BLOCKS + 1) * BLOCK),
        Err(RequestedSizeError::TooLarge)
    );
    assert_eq!(device.requested_size(), BLOCK);
}

#[test]
fn geometry_rejects_every_unsupported_shape() {
    let ok = |block, addr, size| MemGeometry::new(block, addr, size, 0);
    assert!(ok(BLOCK, BASE, BLOCK).is_ok());
    assert_eq!(ok(BLOCK + 1, BASE, BLOCK), Err(GeometryError::BadBlockSize));
    assert_eq!(ok(MIB, BASE, MIB), Err(GeometryError::BadBlockSize));
    assert_eq!(
        ok(MAX_BLOCK_SIZE * 2, 0, MAX_BLOCK_SIZE * 2),
        Err(GeometryError::BadBlockSize)
    );
    assert_eq!(ok(0, 0, 0), Err(GeometryError::BadBlockSize));
    assert_eq!(ok(BLOCK, BASE + MIB, BLOCK), Err(GeometryError::BadRegion));
    assert_eq!(
        ok(BLOCK, u64::MAX - BLOCK + 1, BLOCK * 2),
        Err(GeometryError::BadRegion)
    );
    assert_eq!(ok(BLOCK, BASE, 0), Err(GeometryError::BadRegionSize));
    assert_eq!(
        ok(BLOCK, BASE, BLOCK + MIB),
        Err(GeometryError::BadRegionSize)
    );
    let too_many = BLOCK * (u64::try_from(MAX_BLOCKS).expect("small") + 1);
    assert_eq!(ok(BLOCK, BASE, too_many), Err(GeometryError::BadRegionSize));
    assert!(ok(BLOCK, BASE, too_many - BLOCK).is_ok());
}

#[test]
fn activation_rejects_unoffered_features_and_reset_keeps_plugged_blocks() {
    let mut device = MemDevice::new(geometry(), Box::new(NullBackend));
    assert!(matches!(
        device.activate(MEM_FEATURES | (1 << 40)),
        Err(ActivateError::UnsupportedFeatures { .. })
    ));
    assert!(!device.is_activated());
    device.activate(MEM_FEATURES).expect("activate");
    device.set_requested_size(BLOCK).expect("size");
    device.bitmap.plug(0, 1).expect("plug");
    device.reset();
    assert!(!device.is_activated());
    assert_eq!(device.plugged_size(), BLOCK);
}

#[test]
fn snapshot_round_trips_requested_size_and_bitmap() {
    let (mut rig, mut t, _) = boot();
    request(&mut t, 10);
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Plug, 1, 3).0,
        VIRTIO_MEM_RESP_ACK
    );
    assert_eq!(
        ask(&mut rig, &mut t, RequestKind::Plug, 63, 1).0,
        VIRTIO_MEM_RESP_ACK
    );
    let raw = t.device().snapshot_state();
    assert_eq!(raw.len(), MEM_STATE_HEADER_LEN + 8);
    let mut other = MemDevice::new(geometry(), Box::new(NullBackend));
    other.restore_state(&raw).expect("restore");
    assert_eq!(other.requested_size(), 10 * BLOCK);
    assert_eq!(other.bitmap(), t.device().bitmap());
    assert_eq!(other.snapshot_state(), raw);
}

#[test]
fn hostile_snapshot_records_are_rejected_without_changing_the_device() {
    let mut device = MemDevice::new(geometry(), Box::new(NullBackend));
    device.set_requested_size(2 * BLOCK).expect("size");
    device.bitmap.plug(0, 2).expect("plug");
    let good = device.snapshot_state();
    let mut probe = MemDevice::new(geometry(), Box::new(NullBackend));
    let mut cases: Vec<(&str, Vec<u8>, DeviceStateError)> = Vec::new();
    cases.push(("empty", vec![], DeviceStateError::Malformed));
    cases.push((
        "short",
        good[..good.len() - 1].to_vec(),
        DeviceStateError::Malformed,
    ));
    let mut long = good.clone();
    long.push(0);
    cases.push(("long", long, DeviceStateError::Malformed));
    let mut version = good.clone();
    version[0] = MEM_STATE_VERSION + 1;
    cases.push(("version", version, DeviceStateError::Malformed));
    let mut block = good.clone();
    block[1..9].copy_from_slice(&(BLOCK + 1).to_le_bytes());
    cases.push(("block size", block, DeviceStateError::Malformed));
    let mut huge = good.clone();
    huge[17..25].copy_from_slice(&u64::MAX.to_le_bytes());
    cases.push(("huge region", huge, DeviceStateError::Malformed));
    let mut requested = good.clone();
    requested[25..33].copy_from_slice(&((BLOCKS + 1) * BLOCK).to_le_bytes());
    cases.push((
        "requested too large",
        requested,
        DeviceStateError::Malformed,
    ));
    let mut unaligned = good.clone();
    unaligned[25..33].copy_from_slice(&(BLOCK + 1).to_le_bytes());
    cases.push((
        "requested unaligned",
        unaligned,
        DeviceStateError::Malformed,
    ));
    let mut moved = good.clone();
    moved[9..17].copy_from_slice(&(BASE + BLOCK).to_le_bytes());
    cases.push(("other geometry", moved, DeviceStateError::Incompatible));
    for (name, raw, expected) in cases {
        assert_eq!(probe.restore_state(&raw), Err(expected), "{name}");
        assert_eq!(probe.requested_size(), 0, "{name}");
        assert_eq!(probe.plugged_size(), 0, "{name}");
    }
    assert!(MemState::from_bytes(&good).is_ok());
}

#[test]
fn bitmap_ranges_cross_word_boundaries_and_stay_consistent() {
    let mut bitmap = PluggedBitmap::new(200).expect("bitmap");
    bitmap.plug(60, 10).expect("plug across 64");
    bitmap.plug(128, 72).expect("plug to the end");
    assert_eq!(bitmap.plugged_blocks(), 82);
    assert!(bitmap.is_plugged(63) && bitmap.is_plugged(64) && bitmap.is_plugged(199));
    assert!(!bitmap.is_plugged(59) && !bitmap.is_plugged(70) && !bitmap.is_plugged(200));
    assert_eq!(bitmap.range_state(60, 10), Ok(RangeState::Plugged));
    assert_eq!(bitmap.range_state(70, 58), Ok(RangeState::Unplugged));
    assert_eq!(bitmap.range_state(55, 10), Ok(RangeState::Mixed));
    assert_eq!(bitmap.plug(69, 2), Err(BitmapError::AlreadyPlugged));
    assert_eq!(bitmap.unplug(59, 3), Err(BitmapError::NotPlugged));
    assert_eq!(bitmap.plugged_blocks(), 82);
    bitmap.unplug(62, 4).expect("unplug middle");
    assert_eq!(bitmap.plugged_blocks(), 78);
    let again = PluggedBitmap::from_bytes(200, &bitmap.to_bytes()).expect("decode");
    assert_eq!(again, bitmap);
    bitmap.clear();
    assert_eq!(bitmap.plugged_blocks(), 0);
    assert_eq!(bitmap.range_state(0, 200), Ok(RangeState::Unplugged));
}

#[test]
fn bitmap_rejects_overflow_empty_and_out_of_range_requests() {
    let mut bitmap = PluggedBitmap::new(64).expect("bitmap");
    for (first, count) in [(0, 0), (0, 65), (64, 1), (usize::MAX, 2), (1, usize::MAX)] {
        assert_eq!(bitmap.plug(first, count), Err(BitmapError::BadRange));
        assert_eq!(bitmap.unplug(first, count), Err(BitmapError::BadRange));
        assert_eq!(bitmap.range_state(first, count), Err(BitmapError::BadRange));
    }
    bitmap.plug(0, 64).expect("whole word");
    assert_eq!(bitmap.plugged_blocks(), 64);
    assert_eq!(PluggedBitmap::new(0), Err(BitmapError::BadBlockCount));
    assert_eq!(
        PluggedBitmap::new(MAX_BLOCKS + 1),
        Err(BitmapError::BadBlockCount)
    );
    assert!(PluggedBitmap::new(MAX_BLOCKS).is_ok());
}

#[test]
fn bitmap_decoding_rejects_wrong_length_and_bits_past_the_end() {
    assert_eq!(
        PluggedBitmap::from_bytes(10, &[0]),
        Err(BitmapError::Malformed)
    );
    assert_eq!(
        PluggedBitmap::from_bytes(10, &[0; 3]),
        Err(BitmapError::Malformed)
    );
    assert_eq!(
        PluggedBitmap::from_bytes(10, &[0, 0b100]),
        Err(BitmapError::Malformed)
    );
    let ok = PluggedBitmap::from_bytes(10, &[0xff, 0b11]).expect("exact");
    assert_eq!(ok.plugged_blocks(), 10);
    assert_eq!(
        PluggedBitmap::from_bytes(0, &[]),
        Err(BitmapError::BadBlockCount)
    );
}

#[test]
fn many_random_requests_keep_the_bitmap_and_counts_consistent() {
    let (mut rig, mut t, _) = boot();
    request(&mut t, BLOCKS);
    let mut shadow = [false; 64];
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..120 {
        let first = next() % BLOCKS;
        let count = next() % 6 + 1;
        let plug = next() % 2 == 0;
        let span = first as usize..(first + count).min(BLOCKS) as usize;
        let fits = first + count <= BLOCKS;
        let all = |value: bool| fits && shadow[span.clone()].iter().all(|b| *b == value);
        let kind = if plug {
            RequestKind::Plug
        } else {
            RequestKind::Unplug
        };
        let (answer, _) = ask(
            &mut rig,
            &mut t,
            kind,
            first,
            u16::try_from(count).expect("small"),
        );
        if (plug && all(false)) || (!plug && all(true)) {
            assert_eq!(answer, VIRTIO_MEM_RESP_ACK);
            shadow[span].fill(plug);
        } else {
            assert_eq!(answer, VIRTIO_MEM_RESP_ERROR);
        }
        let plugged = shadow.iter().filter(|b| **b).count() as u64;
        assert_eq!(t.device().plugged_size(), plugged * BLOCK);
    }
}

#[test]
fn snapshot_with_a_bit_past_a_partial_last_word_is_rejected() {
    let small = MemGeometry::new(BLOCK, BASE, BLOCK * 10, 0).expect("geometry");
    let mut device = MemDevice::new(small, Box::new(NullBackend));
    device.set_requested_size(BLOCK).expect("size");
    let good = device.snapshot_state();
    assert_eq!(good.len(), MEM_STATE_HEADER_LEN + 2);
    let mut stray = good.clone();
    let last = stray.len() - 1;
    stray[last] = 0b100;
    assert_eq!(
        device.restore_state(&stray),
        Err(DeviceStateError::Malformed)
    );
    assert_eq!(device.restore_state(&good), Ok(()));
}
