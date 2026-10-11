//! A model guest driver for the memory device, shared by the hostile, bounds, wire, lifecycle
//! and model-based test files.
//!
//! It talks to the device only through the transport's register interface and descriptor
//! chains, the way a real `virtio_mem` driver does. It never reaches into device internals to
//! make a request, so a test that passes here passes for a guest.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use super::backing::RecordingBacking;
use super::{BLOCK_SIZE, MEM_FEATURES, MemDevice, REQUEST_LEN, RESPONSE_LEN};
use crate::virtio::devices::harness::{GuestRig, Seg};
use crate::virtio::devices::service::{ServiceError, ServiceReport, service_queue};
use crate::virtio::transport::MmioTransport;

/// Guest physical address the test region starts at (above 4 GiB, block aligned).
pub(super) const REGION: u64 = 0x1_0000_0000;

pub(super) const PLUG: u16 = 0;
pub(super) const UNPLUG: u16 = 1;
pub(super) const UNPLUG_ALL: u16 = 2;
pub(super) const STATE: u16 = 3;

pub(super) const ACK: u16 = 0;
pub(super) const NACK: u16 = 1;
pub(super) const ERROR: u16 = 3;

pub(super) const ST_PLUGGED: u16 = 0;
pub(super) const ST_UNPLUGGED: u16 = 1;
pub(super) const ST_MIXED: u16 = 2;

/// One decoded response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Answer {
    pub kind: u16,
    pub state: u16,
    /// The used length the device published for the chain.
    pub used: u32,
}

pub(super) struct Rig {
    pub guest: GuestRig,
    pub t: MmioTransport<MemDevice>,
    pub backed: Arc<Mutex<Vec<(u64, u64)>>>,
    pub discarded: Arc<Mutex<Vec<(u64, u64)>>>,
    pub fail: Arc<AtomicBool>,
}

impl Rig {
    /// A booted device with `blocks` blocks of region and nothing requested.
    pub(super) fn new(blocks: u64) -> Self {
        let backing = RecordingBacking::default();
        let backed = Arc::clone(&backing.plugged);
        let discarded = Arc::clone(&backing.discarded);
        let fail = Arc::clone(&backing.fail);
        let device =
            MemDevice::new(REGION, blocks * BLOCK_SIZE, Box::new(backing)).expect("device");
        let guest = GuestRig::new(&[16]);
        let mut t = MmioTransport::new(device).expect("transport");
        guest.init(&mut t, MEM_FEATURES);
        Self {
            guest,
            t,
            backed,
            discarded,
            fail,
        }
    }

    /// The host asks for `blocks` blocks in total.
    pub(super) fn want(&mut self, blocks: u64) {
        self.t
            .device_mut()
            .set_requested_size(blocks * BLOCK_SIZE)
            .expect("size");
        self.t.signal_config_change();
    }

    /// Guest address of block `index`.
    pub(super) const fn at(index: u64) -> u64 {
        REGION + index * BLOCK_SIZE
    }

    pub(super) fn plugged(&self) -> u64 {
        self.t.device().plugged_size() / BLOCK_SIZE
    }

    pub(super) fn backs(&self) -> Vec<(u64, u64)> {
        self.backed.lock().expect("log").clone()
    }

    pub(super) fn discards(&self) -> Vec<(u64, u64)> {
        self.discarded.lock().expect("log").clone()
    }

    /// Sends one well-formed request and returns the answer.
    pub(super) fn ask(&mut self, kind: u16, addr: u64, blocks: u16) -> Answer {
        self.ask_raw(&encode(kind, addr, blocks))
    }

    /// Sends exactly these 24 bytes as a well-formed chain.
    pub(super) fn ask_raw(&mut self, raw: &[u8; REQUEST_LEN]) -> Answer {
        let body = self.guest.alloc(raw);
        let reply = self.guest.alloc(&[0xa5; RESPONSE_LEN]);
        let head = u32::try_from(REQUEST_LEN).expect("small");
        let tail = u32::try_from(RESPONSE_LEN).expect("small");
        self.guest
            .submit(0, &[Seg::readable(body, head), Seg::writable(reply, tail)]);
        let before = self.guest.used_idx(0);
        let report = self.serve().expect("service never faults");
        assert_eq!(
            report.completed, 1,
            "a well-formed chain is always answered"
        );
        let slot = before;
        let raw = self.guest.read(reply, RESPONSE_LEN);
        Answer {
            kind: u16::from_le_bytes([raw[0], raw[1]]),
            state: u16::from_le_bytes([raw[8], raw[9]]),
            used: self.guest.used_elem(0, slot).1,
        }
    }

    /// Rings the doorbell and services the queue.
    pub(super) fn serve(&mut self) -> Result<ServiceReport, ServiceError> {
        self.guest.notify(&mut self.t, 0);
        service_queue(&mut self.t, &self.guest.mem, 0, 64)
    }
}

/// The 24-byte wire request: type, 6 reserved, address, block count, 6 reserved.
pub(super) fn encode(kind: u16, addr: u64, blocks: u16) -> [u8; REQUEST_LEN] {
    let mut raw = [0_u8; REQUEST_LEN];
    raw[0..2].copy_from_slice(&kind.to_le_bytes());
    raw[8..16].copy_from_slice(&addr.to_le_bytes());
    raw[16..18].copy_from_slice(&blocks.to_le_bytes());
    raw
}
