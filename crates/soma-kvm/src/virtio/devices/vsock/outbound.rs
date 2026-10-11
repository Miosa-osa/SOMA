//! Host-to-guest packet selection for [`VsockDevice`]: queued control
//! packets first, then a pending orderly shutdown, then credit-bounded data,
//! then a coalesced credit update.

use super::connection::HostEndpoint;
use super::credit::HOST_BUF_ALLOC;
use super::packet::{
    HOST_CID, SOMA_CONTROL_PORT, VSOCK_OP_CREDIT_UPDATE, VSOCK_OP_RW, VSOCK_OP_SHUTDOWN,
    VSOCK_SHUTDOWN_RCV, VSOCK_SHUTDOWN_SEND, VSOCK_TYPE_STREAM, VsockHeader,
};
use super::{EVENT_LIMITS, MAX_OUTBOUND_PACKETS, PACKET_LIMITS, VSOCK_EVENT_QUEUE, VsockDevice};
use crate::virtio::queue::chain::ChainLimits;

/// A queued host-to-guest control packet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Outbound {
    pub src_port: u32,
    pub op: u16,
    pub dst_port: u32,
    pub flags: u32,
}

impl VsockDevice {
    pub(super) fn has_outbound(&self) -> bool {
        let wants = |endpoint: &HostEndpoint| {
            endpoint.can_send_data()
                || endpoint.shutdown_ready()
                || (endpoint.is_open() && endpoint.credit_update_due())
        };
        !self.outbound.is_empty()
            || self.endpoint.as_ref().is_some_and(wants)
            || self
                .streams
                .iter()
                .any(|stream| stream.endpoint.as_ref().is_some_and(wants))
    }

    /// Queues a packet from `src_port` to the guest; dropped when the queue is full.
    pub(super) fn queue(&mut self, src_port: u32, op: u16, dst_port: u32, flags: u32) {
        if self.outbound.len() < MAX_OUTBOUND_PACKETS {
            self.outbound.push_back(Outbound {
                src_port,
                op,
                dst_port,
                flags,
            });
        }
    }

    /// Builds the next packet for the guest with at most `cap` payload bytes.
    ///
    /// Queued control packets go first, then the control connection, then the streams in
    /// rotation, so a busy stream can never delay a command and no stream starves another.
    pub(super) fn next_outbound(&mut self, cap: usize) -> Option<(VsockHeader, Vec<u8>)> {
        if let Some(control) = self.outbound.pop_front() {
            let credit = self.credit_of(control.src_port);
            let mut header = self.blank_header(control.src_port, credit);
            header.op = control.op;
            header.dst_port = control.dst_port;
            header.flags = control.flags;
            return Some((header, Vec::new()));
        }
        if let Some(endpoint) = self.endpoint.as_mut()
            && let Some(packet) = packet_for(endpoint, cap)
        {
            return Some(self.finish(SOMA_CONTROL_PORT, packet));
        }
        let count = self.streams.len();
        for step in 0..count {
            let index = (self.next_stream + step) % count;
            let host_port = self.streams[index].host_port;
            let Some(endpoint) = self.streams[index].endpoint.as_mut() else {
                continue;
            };
            if let Some(packet) = packet_for(endpoint, cap) {
                self.next_stream = (index + 1) % count;
                return Some(self.finish(host_port, packet));
            }
        }
        None
    }

    /// The credit fields of the connection a queued packet belongs to, or the empty window.
    fn credit_of(&self, src_port: u32) -> (u32, u32) {
        let endpoint = if src_port == SOMA_CONTROL_PORT {
            self.endpoint.as_ref()
        } else {
            self.streams
                .iter()
                .find(|stream| stream.host_port == src_port)
                .and_then(|stream| stream.endpoint.as_ref())
        };
        endpoint.map_or((HOST_BUF_ALLOC, 0), HostEndpoint::local_credit)
    }

    fn blank_header(&self, src_port: u32, credit: (u32, u32)) -> VsockHeader {
        VsockHeader {
            src_cid: HOST_CID,
            dst_cid: self.guest_cid,
            src_port,
            dst_port: 0,
            len: 0,
            ty: VSOCK_TYPE_STREAM,
            op: 0,
            flags: 0,
            buf_alloc: credit.0,
            fwd_cnt: credit.1,
        }
    }

    fn finish(&self, src_port: u32, packet: Packet) -> (VsockHeader, Vec<u8>) {
        let mut header = self.blank_header(src_port, packet.credit);
        header.dst_port = packet.dst_port;
        header.op = packet.op;
        header.flags = packet.flags;
        header.len = u32::try_from(packet.payload.len()).unwrap_or(u32::MAX);
        (header, packet.payload)
    }

    pub(super) const fn chain_limits_for(queue: u16) -> ChainLimits {
        if queue == VSOCK_EVENT_QUEUE {
            EVENT_LIMITS
        } else {
            PACKET_LIMITS
        }
    }

    pub(super) fn pop_event(&mut self) -> Option<u32> {
        self.events.pop_front()
    }

    pub(super) const fn count_rx_dropped(&mut self) {
        self.counters.rx_dropped = self.counters.rx_dropped.saturating_add(1);
    }

    pub(super) const fn count_rx_packet(&mut self) {
        self.counters.rx_packets = self.counters.rx_packets.saturating_add(1);
    }
}

/// One packet an endpoint wants to send, before the device stamps the addresses on it.
struct Packet {
    op: u16,
    dst_port: u32,
    flags: u32,
    credit: (u32, u32),
    payload: Vec<u8>,
}

/// The next packet one endpoint has for the guest: shutdown, then credit-bounded data, then a
/// coalesced credit update.
fn packet_for(endpoint: &mut HostEndpoint, cap: usize) -> Option<Packet> {
    let dst_port = endpoint.peer_port();
    if endpoint.shutdown_ready() {
        endpoint.mark_shutdown_sent();
        return Some(Packet {
            op: VSOCK_OP_SHUTDOWN,
            dst_port,
            flags: VSOCK_SHUTDOWN_RCV | VSOCK_SHUTDOWN_SEND,
            credit: endpoint.local_credit(),
            payload: Vec::new(),
        });
    }
    if endpoint.can_send_data() && cap > 0 {
        let payload = endpoint.take_to_guest(cap);
        return Some(Packet {
            op: VSOCK_OP_RW,
            dst_port,
            flags: 0,
            credit: endpoint.local_credit(),
            payload,
        });
    }
    if endpoint.is_open() && endpoint.credit_update_due() {
        endpoint.clear_credit_update();
        return Some(Packet {
            op: VSOCK_OP_CREDIT_UPDATE,
            dst_port,
            flags: 0,
            credit: endpoint.local_credit(),
            payload: Vec::new(),
        });
    }
    None
}
