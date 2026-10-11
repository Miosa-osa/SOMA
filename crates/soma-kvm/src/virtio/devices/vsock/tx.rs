//! Guest-to-host packet handling for [`VsockDevice`].

use super::VsockDevice;
use super::connection::HostEndpoint;
use super::packet::{
    SOMA_CONTROL_PORT, VSOCK_HDR_LEN, VSOCK_OP_CREDIT_REQUEST, VSOCK_OP_CREDIT_UPDATE,
    VSOCK_OP_REQUEST, VSOCK_OP_RESPONSE, VSOCK_OP_RST, VSOCK_OP_RW, VSOCK_OP_SHUTDOWN, VsockHeader,
    parse_tx,
};
use crate::virtio::devices::segments::read_readable;
use crate::virtio::devices::service::DeviceFault;
use crate::virtio::guest_memory::GuestMemory;
use crate::virtio::queue::chain::DescriptorChain;

/// Which host connection a guest packet is addressed to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Target {
    Control,
    Stream(usize),
}

impl VsockDevice {
    /// Validates one transmit chain and applies it to the connection state.
    ///
    /// Every rejection is a counter tick and, where the specification says
    /// so, a queued `RST`; only an inconsistency in validated guest memory
    /// is a fault.
    pub(super) fn handle_tx<M: GuestMemory + ?Sized>(
        &mut self,
        chain: &DescriptorChain,
        mem: &M,
    ) -> Result<(), DeviceFault> {
        let Ok(header) = parse_tx(mem, chain, self.guest_cid) else {
            {
                self.reject();
                return Ok(());
            }
        };
        self.counters.tx_packets = self.counters.tx_packets.saturating_add(1);
        let Some(target) = self.target_of(header.dst_port) else {
            if header.op == VSOCK_OP_REQUEST {
                self.send_rst(SOMA_CONTROL_PORT, header.src_port);
            }
            {
                self.reject();
                return Ok(());
            }
        };
        let port = self.host_port(target);
        if header.op == VSOCK_OP_REQUEST {
            self.on_request(target, header);
            return Ok(());
        }
        let connected = self
            .slot(target)
            .as_ref()
            .is_some_and(|endpoint| endpoint.is_open() && endpoint.peer_port() == header.src_port);
        if !connected {
            if header.op != VSOCK_OP_RST {
                self.send_rst(port, header.src_port);
            }
            {
                self.reject();
                return Ok(());
            }
        }
        self.on_connected(target, header, chain, mem)
    }

    /// The connection a destination port names: the control port, or an armed stream port.
    fn target_of(&self, dst_port: u32) -> Option<Target> {
        if dst_port == SOMA_CONTROL_PORT {
            return Some(Target::Control);
        }
        self.streams
            .iter()
            .position(|stream| stream.host_port == dst_port)
            .map(Target::Stream)
    }

    fn host_port(&self, target: Target) -> u32 {
        match target {
            Target::Control => SOMA_CONTROL_PORT,
            Target::Stream(index) => self.streams[index].host_port,
        }
    }

    fn slot(&mut self, target: Target) -> &mut Option<HostEndpoint> {
        match target {
            Target::Control => &mut self.endpoint,
            Target::Stream(index) => &mut self.streams[index].endpoint,
        }
    }

    /// Counts a per-packet rejection; never a fault.
    fn reject(&mut self) {
        self.counters.rejected = self.counters.rejected.saturating_add(1);
    }

    fn send_rst(&mut self, host_port: u32, guest_port: u32) {
        self.queue(host_port, VSOCK_OP_RST, guest_port, 0);
        self.counters.rst_sent = self.counters.rst_sent.saturating_add(1);
    }

    /// Resets the connection to `guest_port` and drops the endpoint's open state.
    fn abort(&mut self, target: Target, guest_port: u32) {
        if let Some(endpoint) = self.slot(target).as_mut() {
            endpoint.close();
        }
        let port = self.host_port(target);
        self.send_rst(port, guest_port);
    }

    fn on_request(&mut self, target: Target, header: VsockHeader) {
        let port = self.host_port(target);
        if self
            .slot(target)
            .as_ref()
            .is_some_and(HostEndpoint::is_open)
        {
            self.send_rst(port, header.src_port);
            return self.reject();
        }
        self.generation = self.generation.wrapping_add(1);
        let accepted = HostEndpoint::accept(
            header.src_port,
            self.generation,
            header.buf_alloc,
            header.fwd_cnt,
        );
        let Ok(endpoint) = accepted else {
            self.send_rst(port, header.src_port);
            return self.reject();
        };
        *self.slot(target) = Some(endpoint);
        self.queue(port, VSOCK_OP_RESPONSE, header.src_port, 0);
        self.counters.accepted = self.counters.accepted.saturating_add(1);
    }

    fn on_connected<M: GuestMemory + ?Sized>(
        &mut self,
        target: Target,
        header: VsockHeader,
        chain: &DescriptorChain,
        mem: &M,
    ) -> Result<(), DeviceFault> {
        let Some(endpoint) = self.slot(target).as_mut() else {
            return Ok(());
        };
        if endpoint
            .update_peer_credit(header.buf_alloc, header.fwd_cnt)
            .is_err()
        {
            self.abort(target, header.src_port);
            {
                self.reject();
                return Ok(());
            }
        }
        match header.op {
            VSOCK_OP_RST => endpoint.close(),
            VSOCK_OP_SHUTDOWN => {
                endpoint.peer_shutdown(header.flags);
                if endpoint.peer_fully_shut() {
                    self.abort(target, header.src_port);
                }
            }
            VSOCK_OP_RW => {
                let len = usize::try_from(header.len).map_err(|_| DeviceFault::Protocol)?;
                let mut payload = vec![0u8; len];
                let skip = u64::try_from(VSOCK_HDR_LEN).map_err(|_| DeviceFault::Protocol)?;
                if read_readable(mem, chain, skip, &mut payload)? != len {
                    return Err(DeviceFault::Protocol);
                }
                if endpoint.push_from_guest(&payload).is_err() {
                    self.abort(target, header.src_port);
                    {
                        self.reject();
                        return Ok(());
                    }
                }
            }
            VSOCK_OP_CREDIT_UPDATE => {}
            VSOCK_OP_CREDIT_REQUEST => {
                let port = self.host_port(target);
                self.queue(port, VSOCK_OP_CREDIT_UPDATE, header.src_port, 0);
            }
            VSOCK_OP_RESPONSE => {
                self.abort(target, header.src_port);
                {
                    self.reject();
                    return Ok(());
                }
            }
            _ => {
                self.reject();
                return Ok(());
            }
        }
        Ok(())
    }
}
