//! Host-to-guest packet selection for [`VsockDevice`]: queued control
//! packets first, then a pending orderly shutdown, then credit-bounded data,
//! then a coalesced credit update.

use super::connection::HostEndpoint;
use super::credit::HOST_BUF_ALLOC;
use super::packet::{
    HOST_CID, SOMA_CONTROL_PORT, VSOCK_OP_CREDIT_UPDATE, VSOCK_OP_RW, VSOCK_OP_SHUTDOWN,
    VSOCK_SHUTDOWN_RCV, VSOCK_SHUTDOWN_SEND, VSOCK_TYPE_STREAM, VsockHeader,
};
use super::{EVENT_LIMITS, PACKET_LIMITS, VSOCK_EVENT_QUEUE, VsockDevice};
use crate::virtio::queue::chain::ChainLimits;

impl VsockDevice {
    pub(super) fn has_outbound(&self) -> bool {
        !self.outbound.is_empty()
            || self.endpoint.as_ref().is_some_and(|endpoint| {
                endpoint.can_send_data()
                    || endpoint.shutdown_ready()
                    || (endpoint.is_open() && endpoint.credit_update_due())
            })
    }

    /// Builds the next packet for the guest with at most `cap` payload bytes.
    pub(super) fn next_outbound(&mut self, cap: usize) -> Option<(VsockHeader, Vec<u8>)> {
        let (buf_alloc, fwd_cnt) = self
            .endpoint
            .as_ref()
            .map_or((HOST_BUF_ALLOC, 0), HostEndpoint::local_credit);
        let mut header = VsockHeader {
            src_cid: HOST_CID,
            dst_cid: self.guest_cid,
            src_port: SOMA_CONTROL_PORT,
            dst_port: 0,
            len: 0,
            ty: VSOCK_TYPE_STREAM,
            op: 0,
            flags: 0,
            buf_alloc,
            fwd_cnt,
        };
        if let Some(control) = self.outbound.pop_front() {
            header.op = control.op;
            header.dst_port = control.dst_port;
            header.flags = control.flags;
            return Some((header, Vec::new()));
        }
        let endpoint = self.endpoint.as_mut()?;
        header.dst_port = endpoint.peer_port();
        if endpoint.shutdown_ready() {
            endpoint.mark_shutdown_sent();
            header.op = VSOCK_OP_SHUTDOWN;
            header.flags = VSOCK_SHUTDOWN_RCV | VSOCK_SHUTDOWN_SEND;
            return Some((header, Vec::new()));
        }
        if endpoint.can_send_data() && cap > 0 {
            let payload = endpoint.take_to_guest(cap);
            header.op = VSOCK_OP_RW;
            header.len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
            let (_, fwd_cnt) = endpoint.local_credit();
            header.fwd_cnt = fwd_cnt;
            return Some((header, payload));
        }
        if endpoint.is_open() && endpoint.credit_update_due() {
            endpoint.clear_credit_update();
            header.op = VSOCK_OP_CREDIT_UPDATE;
            return Some((header, Vec::new()));
        }
        None
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
