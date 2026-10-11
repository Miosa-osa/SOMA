//! Data streams beside the control connection.
//!
//! The control connection carries one request and one answer at a time, so anything that moves
//! bytes for as long as a browser tab is open needs its own connection. The host arms a stream
//! port, tells the agent through the control connection to dial it, and the agent's connection
//! lands here. Only an armed port accepts a connection, only once, and only a small fixed number
//! of ports exist, so a guest cannot open streams the host did not ask for and cannot hold more
//! than the host armed.
//!
//! Like the control connection, a stream lives only in this model: it never enters the snapshot
//! record, and a restore clears every one.

use super::VsockDevice;
use super::connection::HostEndpoint;
use super::packet::SOMA_CONTROL_PORT;

/// The most streams that can be armed or open at once.
pub const MAX_STREAMS: usize = 8;
const STREAM_PORTS: u32 = 8;
/// The first stream port; the ports are consecutive from here.
pub const SOMA_STREAM_PORT_BASE: u32 = SOMA_CONTROL_PORT + 1;

/// One armed stream port, and the guest connection it accepted, if any.
#[derive(Debug)]
pub struct Stream {
    pub(super) host_port: u32,
    pub(super) endpoint: Option<HostEndpoint>,
}

impl VsockDevice {
    /// Arms the lowest free stream port and returns it, or `None` when every port is in use.
    ///
    /// The guest is expected to connect to the returned port. Until it does, and after the host
    /// closes the stream, nothing is accepted there.
    pub fn arm_stream(&mut self) -> Option<u32> {
        if self.streams.len() >= MAX_STREAMS {
            return None;
        }
        let port = (SOMA_STREAM_PORT_BASE..SOMA_STREAM_PORT_BASE + STREAM_PORTS)
            .find(|port| self.streams.iter().all(|stream| stream.host_port != *port))?;
        self.streams.push(Stream {
            host_port: port,
            endpoint: None,
        });
        Some(port)
    }

    /// The guest connection on an armed stream port, once the guest has connected.
    pub fn stream(&mut self, port: u32) -> Option<&mut HostEndpoint> {
        self.streams
            .iter_mut()
            .find(|stream| stream.host_port == port)?
            .endpoint
            .as_mut()
    }

    /// Whether `port` is armed, connected or not.
    #[must_use]
    pub fn is_stream_armed(&self, port: u32) -> bool {
        self.streams.iter().any(|stream| stream.host_port == port)
    }

    /// Disarms `port`, resetting the connection if one is open, and frees the port.
    pub fn close_stream(&mut self, port: u32) {
        let Some(index) = self
            .streams
            .iter()
            .position(|stream| stream.host_port == port)
        else {
            return;
        };
        let stream = self.streams.remove(index);
        if let Some(endpoint) = stream.endpoint
            && endpoint.is_open()
        {
            self.queue(port, super::packet::VSOCK_OP_RST, endpoint.peer_port(), 0);
            self.counters.rst_sent = self.counters.rst_sent.saturating_add(1);
        }
        if self.next_stream >= self.streams.len() {
            self.next_stream = 0;
        }
    }
}
