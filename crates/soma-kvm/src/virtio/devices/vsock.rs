//! virtio-vsock (device id 19): the sole guest-control transport.
//!
//! Receive, transmit, and event queues, no device-specific features, and a
//! host endpoint that accepts exactly one stream connection at a time on the
//! fixed SOMA control port. Connection, credit, and buffered bytes live only
//! in this model and never enter the snapshot record.

pub mod connection;
pub mod credit;
mod outbound;
pub mod packet;
pub mod rx;
pub mod state;
mod tx;

use std::collections::VecDeque;
use std::fmt;

use crate::virtio::device::{
    ActivateError, ConfigAccessError, DeviceStateError, VIRTIO_F_VERSION_1, VirtioDevice,
};
use crate::virtio::devices::service::{ChainHandler, DeviceFault};
use crate::virtio::guest_memory::GuestMemory;
use crate::virtio::queue::chain::{ChainLimits, DescriptorChain};
use connection::HostEndpoint;
use packet::{VSOCK_EVENT_TRANSPORT_RESET, VSOCK_OP_RST};

/// Exact feature allowlist: the common modern bit only.
pub const VSOCK_FEATURES: u64 = VIRTIO_F_VERSION_1;
pub const VSOCK_RX_QUEUE: u16 = 0;
pub const VSOCK_TX_QUEUE: u16 = 1;
pub const VSOCK_EVENT_QUEUE: u16 = 2;
/// Queue count and maximum sizes from the device-surface table.
pub const VSOCK_QUEUE_MAX: [u16; 3] = [256, 256, 64];
/// Configuration space: `guest_cid` as a 64-bit value.
pub const VSOCK_CONFIG_LEN: usize = 8;
/// Largest number of queued control packets before new ones are dropped.
pub const MAX_OUTBOUND_PACKETS: usize = 64;
/// Largest number of queued transport events.
pub const MAX_PENDING_EVENTS: usize = 8;
/// Smallest assignable guest CID; 0, 1, and 2 are reserved.
pub const MIN_GUEST_CID: u64 = 3;
/// `VMADDR_CID_ANY`; never assignable.
pub const CID_ANY: u64 = 0xffff_ffff;
/// Header plus [`MAX_PAYLOAD_LEN`]; spelled out because `From` is not `const`.
const PACKET_LIMITS: ChainLimits = ChainLimits {
    max_descriptors: 256,
    max_bytes: 44 + (1 << 16),
};
const EVENT_LIMITS: ChainLimits = ChainLimits {
    max_descriptors: 64,
    max_bytes: 4096,
};

/// Why a vsock device could not be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VsockConfigError {
    /// The CID is reserved or `VMADDR_CID_ANY`.
    InvalidCid { cid: u64 },
}

impl fmt::Display for VsockConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "vsock device rejected: {self:?}")
    }
}

impl std::error::Error for VsockConfigError {}

/// Saturating counters; never payload bytes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VsockCounters {
    pub accepted: u32,
    pub rejected: u32,
    pub rst_sent: u32,
    pub tx_packets: u32,
    pub rx_packets: u32,
    pub rx_dropped: u32,
}

/// A queued host-to-guest control packet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Outbound {
    pub op: u16,
    pub dst_port: u32,
    pub flags: u32,
}

/// The vsock device model.
pub struct VsockDevice {
    guest_cid: u64,
    generation: u32,
    endpoint: Option<HostEndpoint>,
    outbound: VecDeque<Outbound>,
    events: VecDeque<u32>,
    activated: bool,
    counters: VsockCounters,
}

impl VsockDevice {
    /// A device for the assigned guest CID with no connection.
    ///
    /// # Errors
    /// Rejects a reserved CID.
    pub fn new(guest_cid: u64) -> Result<Self, VsockConfigError> {
        Self::check_cid(guest_cid)?;
        Ok(Self {
            guest_cid,
            generation: 0,
            endpoint: None,
            outbound: VecDeque::new(),
            events: VecDeque::new(),
            activated: false,
            counters: VsockCounters::default(),
        })
    }

    const fn check_cid(cid: u64) -> Result<(), VsockConfigError> {
        if cid < MIN_GUEST_CID || cid >= CID_ANY {
            return Err(VsockConfigError::InvalidCid { cid });
        }
        Ok(())
    }

    /// Validates a context identifier before an authority-transfer transaction mutates devices.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub(crate) const fn validate_guest_cid(cid: u64) -> Result<(), VsockConfigError> {
        Self::check_cid(cid)
    }

    #[must_use]
    pub const fn guest_cid(&self) -> u64 {
        self.guest_cid
    }

    /// Assigns a fresh CID; the caller signals the configuration change.
    ///
    /// # Errors
    /// Rejects a reserved CID.
    pub fn set_guest_cid(&mut self, cid: u64) -> Result<(), VsockConfigError> {
        Self::check_cid(cid)?;
        self.guest_cid = cid;
        Ok(())
    }

    /// The current connection, open or closed with unread bytes, if any.
    pub const fn endpoint(&mut self) -> Option<&mut HostEndpoint> {
        self.endpoint.as_mut()
    }

    /// Drops the endpoint; an open connection is reset first.
    pub fn close_endpoint(&mut self) {
        if let Some(endpoint) = self.endpoint.take()
            && endpoint.is_open()
        {
            self.queue(VSOCK_OP_RST, endpoint.peer_port(), 0);
            self.counters.rst_sent = self.counters.rst_sent.saturating_add(1);
        }
    }

    #[must_use]
    pub const fn counters(&self) -> VsockCounters {
        self.counters
    }

    #[must_use]
    pub const fn generation(&self) -> u32 {
        self.generation
    }

    #[must_use]
    pub const fn is_activated(&self) -> bool {
        self.activated
    }

    /// Queued transport events not yet delivered.
    #[must_use]
    pub fn pending_events(&self) -> usize {
        self.events.len()
    }

    /// Whether the model holds no connection, packet, or event: the only
    /// state in which capture is permitted.
    #[must_use]
    pub fn is_quiescent(&self) -> bool {
        self.endpoint.is_none() && self.outbound.is_empty() && self.events.is_empty()
    }

    pub(super) fn queue(&mut self, op: u16, dst_port: u32, flags: u32) {
        if self.outbound.len() < MAX_OUTBOUND_PACKETS {
            self.outbound.push_back(Outbound {
                op,
                dst_port,
                flags,
            });
        }
    }

    /// Clears every transient state and bumps the generation.
    fn clear_transient(&mut self) {
        self.endpoint = None;
        self.outbound.clear();
        self.events.clear();
        self.generation = self.generation.wrapping_add(1);
    }

    /// Called by the state record after a validated restore.
    pub(super) fn after_restore(&mut self) {
        self.clear_transient();
        self.events.push_back(VSOCK_EVENT_TRANSPORT_RESET);
    }
}

impl ChainHandler for VsockDevice {
    fn chain_limits(&self, queue: u16) -> ChainLimits {
        Self::chain_limits_for(queue)
    }

    /// Transmit only; receive and event buffers are filled by the `rx` module.
    fn handle_chain<M: GuestMemory + ?Sized>(
        &mut self,
        queue: u16,
        chain: &DescriptorChain,
        mem: &M,
    ) -> Result<u32, DeviceFault> {
        if queue != VSOCK_TX_QUEUE {
            return Err(DeviceFault::Protocol);
        }
        self.handle_tx(chain, mem)?;
        Ok(0)
    }
}

impl VirtioDevice for VsockDevice {
    fn device_id(&self) -> u32 {
        packet::VIRTIO_VSOCK_DEVICE_ID
    }

    fn feature_allowlist(&self) -> u64 {
        VSOCK_FEATURES
    }

    fn queue_max_sizes(&self) -> &[u16] {
        &VSOCK_QUEUE_MAX
    }

    fn config_len(&self) -> usize {
        VSOCK_CONFIG_LEN
    }

    fn read_config(&self, offset: usize, buf: &mut [u8]) -> Result<(), ConfigAccessError> {
        let raw = self.guest_cid.to_le_bytes();
        let end = offset
            .checked_add(buf.len())
            .filter(|end| *end <= VSOCK_CONFIG_LEN)
            .ok_or(ConfigAccessError::OutOfBounds)?;
        buf.copy_from_slice(&raw[offset..end]);
        Ok(())
    }

    fn write_config(&mut self, _offset: usize, _data: &[u8]) -> Result<(), ConfigAccessError> {
        Err(ConfigAccessError::ReadOnly)
    }

    fn activate(&mut self, negotiated_features: u64) -> Result<(), ActivateError> {
        if negotiated_features & !VSOCK_FEATURES != 0 {
            return Err(ActivateError::UnsupportedFeatures {
                negotiated: negotiated_features,
            });
        }
        self.activated = true;
        Ok(())
    }

    fn reset(&mut self) {
        self.activated = false;
        self.clear_transient();
    }

    fn snapshot_state(&self) -> Vec<u8> {
        state::VsockState::capture(self).to_bytes().to_vec()
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), DeviceStateError> {
        state::VsockState::from_bytes(bytes)?.apply(self)
    }
}

#[cfg(test)]
pub(crate) mod guest_driver;
#[cfg(test)]
mod hostile_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod tests;
