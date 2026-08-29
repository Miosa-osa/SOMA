//! A model guest vsock driver for tests: receive and event buffer pools,
//! packet submission, and delivery collection through the transport.

use std::collections::HashMap;

use super::credit::HOST_BUF_ALLOC;
use super::packet::*;
use super::rx::{deliver_events, deliver_rx};
use super::*;

use crate::virtio::devices::harness::{GuestRig, Seg};
use crate::virtio::devices::service::{ServiceReport, service_queue};
use crate::virtio::transport::MmioTransport;

pub(super) const CID: u64 = 1234;
pub(super) const GUEST_PORT: u32 = 40_000;
pub(super) const GUEST_BUF: u32 = 4096;

/// A model guest driver with receive and event buffer pools.
pub(super) struct GuestVsock {
    pub rig: GuestRig,
    pub t: MmioTransport<VsockDevice>,
    rx: HashMap<u16, u64>,
    rx_cursor: u16,
    ev: HashMap<u16, u64>,
    ev_cursor: u16,
    pub empty_returns: u32,
}

impl GuestVsock {
    pub(super) fn boot() -> Self {
        let rig = GuestRig::new(&[32, 32, 8]);
        let mut t = MmioTransport::new(VsockDevice::new(CID).expect("device")).expect("transport");
        rig.init(&mut t, VSOCK_FEATURES);
        Self {
            rig,
            t,
            rx: HashMap::new(),
            rx_cursor: 0,
            ev: HashMap::new(),
            ev_cursor: 0,
            empty_returns: 0,
        }
    }

    pub(super) fn device(&mut self) -> &mut VsockDevice {
        self.t.device_mut()
    }

    pub(super) fn post_rx(&mut self, len: u32) {
        let addr = self.rig.alloc_zeroed(len);
        self.post_rx_chain(&[Seg::writable(addr, len)], addr);
    }

    /// Posts an arbitrary chain on the receive queue whose first byte is `addr`.
    pub(super) fn post_rx_chain(&mut self, segments: &[Seg], addr: u64) {
        let head = self.rig.submit(VSOCK_RX_QUEUE, segments);
        self.rx.insert(head, addr);
    }

    pub(super) fn post_event(&mut self) {
        let addr = self.rig.alloc_zeroed(4);
        let head = self
            .rig
            .submit(VSOCK_EVENT_QUEUE, &[Seg::writable(addr, 4)]);
        self.ev.insert(head, addr);
    }

    pub(super) fn send(&mut self, header: VsockHeader, payload: &[u8]) -> ServiceReport {
        let hdr = self.rig.alloc(&header.to_bytes());
        let mut segments = vec![Seg::readable(hdr, 44)];
        if !payload.is_empty() {
            let data = self.rig.alloc(payload);
            segments.push(Seg::readable(
                data,
                u32::try_from(payload.len()).expect("small"),
            ));
        }
        self.send_raw(&segments)
    }

    pub(super) fn send_raw(&mut self, segments: &[Seg]) -> ServiceReport {
        self.rig.submit(VSOCK_TX_QUEUE, segments);
        self.rig.notify(&mut self.t, VSOCK_TX_QUEUE);
        let report = service_queue(&mut self.t, &self.rig.mem, VSOCK_TX_QUEUE, 8).expect("tx");
        assert_eq!(report.completed + report.rejected, 1);
        report
    }

    /// Runs delivery and returns every packet the guest received since last time.
    pub(super) fn recv(&mut self) -> Vec<(VsockHeader, Vec<u8>)> {
        deliver_rx(&mut self.t, &self.rig.mem, 16).expect("deliver");
        let mut packets = Vec::new();
        while self.rx_cursor != self.rig.used_idx(VSOCK_RX_QUEUE) {
            let (head, len) = self.rig.used_elem(VSOCK_RX_QUEUE, self.rx_cursor);
            self.rx_cursor = self.rx_cursor.wrapping_add(1);
            let addr = self.rx[&u16::try_from(head).expect("head")];
            if len < 44 {
                self.empty_returns += 1;
                continue;
            }
            let raw: [u8; 44] = self.rig.read(addr, 44).try_into().expect("header");
            let header = VsockHeader::from_bytes(&raw);
            let payload = self
                .rig
                .read(addr + 44, usize::try_from(len - 44).expect("small"));
            packets.push((header, payload));
        }
        packets
    }

    pub(super) fn events(&mut self) -> Vec<u32> {
        deliver_events(&mut self.t, &self.rig.mem).expect("events");
        let mut events = Vec::new();
        while self.ev_cursor != self.rig.used_idx(VSOCK_EVENT_QUEUE) {
            let (head, len) = self.rig.used_elem(VSOCK_EVENT_QUEUE, self.ev_cursor);
            self.ev_cursor = self.ev_cursor.wrapping_add(1);
            assert_eq!(len, 4);
            let addr = self.ev[&u16::try_from(head).expect("head")];
            let raw: [u8; 4] = self.rig.read(addr, 4).try_into().expect("event");
            events.push(u32::from_le_bytes(raw));
        }
        events
    }
}

pub(super) fn hdr(op: u16, len: u32, flags: u32, fwd_cnt: u32) -> VsockHeader {
    VsockHeader {
        src_cid: CID,
        dst_cid: HOST_CID,
        src_port: GUEST_PORT,
        dst_port: SOMA_CONTROL_PORT,
        len,
        ty: VSOCK_TYPE_STREAM,
        op,
        flags,
        buf_alloc: GUEST_BUF,
        fwd_cnt,
    }
}

pub(super) fn connect(guest: &mut GuestVsock) {
    guest.post_rx(4096);
    guest.send(hdr(VSOCK_OP_REQUEST, 0, 0, 0), &[]);
    let packets = guest.recv();
    assert_eq!(packets.len(), 1, "{packets:?}");
    let (response, payload) = &packets[0];
    assert!(payload.is_empty());
    assert_eq!(response.op, VSOCK_OP_RESPONSE);
    assert_eq!((response.src_cid, response.dst_cid), (HOST_CID, CID));
    assert_eq!(
        (response.src_port, response.dst_port),
        (SOMA_CONTROL_PORT, GUEST_PORT)
    );
    assert_eq!(
        (response.ty, response.buf_alloc, response.fwd_cnt),
        (1, HOST_BUF_ALLOC, 0)
    );
}
