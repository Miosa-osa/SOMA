//! virtio-net (device id 1): one receive queue, one transmit queue, and a
//! preopened frame backend behind a host-controlled link gate.
//!
//! Only `VIRTIO_NET_F_MAC` is offered, so every header is the plain 12-byte
//! `virtio_net_hdr_v1` with all fields zero. The link starts down and is
//! raised only by the host after network repair; while down, transmit
//! frames are dropped and nothing is read from the backend.

pub mod backend;
pub mod frame;
pub mod rx;
pub mod state;

use crate::virtio::device::{
    ActivateError, ConfigAccessError, DeviceStateError, VIRTIO_F_VERSION_1, VirtioDevice,
};
use crate::virtio::devices::segments::read_readable;
use crate::virtio::devices::service::{ChainHandler, DeviceFault};
use crate::virtio::guest_memory::GuestMemory;
use crate::virtio::queue::chain::{ChainLimits, DescriptorChain};
use backend::NetBackend;
use frame::{MAX_FRAME_LEN, VIRTIO_NET_HDR_LEN, validate_tx};

/// Virtio device identifier for a network device.
pub const VIRTIO_NET_DEVICE_ID: u32 = 1;
/// Feature: the configuration space carries a MAC address.
pub const VIRTIO_NET_F_MAC: u64 = 1 << 5;
/// Exact feature allowlist.
pub const NET_FEATURES: u64 = VIRTIO_F_VERSION_1 | VIRTIO_NET_F_MAC;
/// Receive queue index.
pub const NET_RX_QUEUE: u16 = 0;
/// Transmit queue index.
pub const NET_TX_QUEUE: u16 = 1;
/// Queue count and maximum sizes from the device-surface table.
pub const NET_QUEUE_MAX: [u16; 2] = [256, 256];
/// Configuration space: `mac[6]` then `status` (unused without `F_STATUS`).
pub const NET_CONFIG_LEN: usize = 8;
/// Largest receive chain accepted; Linux posts page-sized buffers.
pub const MAX_RX_CHAIN_BYTES: u64 = 1 << 16;
const TX_LIMITS: ChainLimits = ChainLimits {
    max_descriptors: 256,
    max_bytes: (VIRTIO_NET_HDR_LEN + MAX_FRAME_LEN) as u64,
};
const RX_LIMITS: ChainLimits = ChainLimits {
    max_descriptors: 256,
    max_bytes: MAX_RX_CHAIN_BYTES,
};

/// Saturating counters; never frame bytes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NetCounters {
    pub tx_ok: u32,
    pub tx_dropped: u32,
    pub rx_ok: u32,
    pub rx_dropped: u32,
}

/// The network device model.
pub struct NetDevice {
    backend: Box<dyn NetBackend + Send>,
    mac: [u8; 6],
    link_up: bool,
    activated: bool,
    counters: NetCounters,
}

impl NetDevice {
    /// Binds a backend with the Generation's placeholder MAC; link down.
    #[must_use]
    pub fn new(backend: Box<dyn NetBackend + Send>, mac: [u8; 6]) -> Self {
        Self {
            backend,
            mac,
            link_up: false,
            activated: false,
            counters: NetCounters::default(),
        }
    }

    /// Installs the effective MAC; the caller signals the config change.
    pub const fn set_mac(&mut self, mac: [u8; 6]) {
        self.mac = mac;
    }

    #[must_use]
    pub const fn mac(&self) -> [u8; 6] {
        self.mac
    }

    /// Raises or lowers the host-side link gate.
    pub const fn set_link(&mut self, up: bool) {
        self.link_up = up;
    }

    #[must_use]
    pub const fn link_up(&self) -> bool {
        self.link_up
    }

    #[must_use]
    pub const fn counters(&self) -> NetCounters {
        self.counters
    }

    #[must_use]
    pub const fn is_activated(&self) -> bool {
        self.activated
    }

    pub(super) const fn feature_allowlist_value() -> u64 {
        NET_FEATURES
    }

    pub(super) const fn rx_limits() -> ChainLimits {
        RX_LIMITS
    }

    /// Reads one frame from the backend; `Err` means the backend is unusable.
    pub(super) fn receive_frame(&mut self, buf: &mut [u8]) -> Result<Option<usize>, ()> {
        self.backend.receive(buf).map_err(|_| ())
    }

    pub(super) const fn count_rx_ok(&mut self) {
        self.counters.rx_ok = self.counters.rx_ok.saturating_add(1);
    }

    pub(super) const fn count_rx_dropped(&mut self) {
        self.counters.rx_dropped = self.counters.rx_dropped.saturating_add(1);
    }

    fn drop_tx(&mut self) -> u32 {
        self.counters.tx_dropped = self.counters.tx_dropped.saturating_add(1);
        0
    }
}

impl ChainHandler for NetDevice {
    fn chain_limits(&self, queue: u16) -> ChainLimits {
        if queue == NET_TX_QUEUE {
            TX_LIMITS
        } else {
            RX_LIMITS
        }
    }

    /// Transmit only; receive buffers are consumed by [`rx::deliver_rx`].
    fn handle_chain<M: GuestMemory + ?Sized>(
        &mut self,
        queue: u16,
        chain: &DescriptorChain,
        mem: &M,
    ) -> Result<u32, DeviceFault> {
        if queue != NET_TX_QUEUE {
            return Err(DeviceFault::Protocol);
        }
        let Ok(frame_len) = validate_tx(mem, chain) else {
            return Ok(self.drop_tx());
        };
        if !self.link_up {
            return Ok(self.drop_tx());
        }
        let mut frame = [0u8; MAX_FRAME_LEN];
        let hdr = u64::try_from(VIRTIO_NET_HDR_LEN).map_err(|_| DeviceFault::Protocol)?;
        if read_readable(mem, chain, hdr, &mut frame[..frame_len])? != frame_len {
            return Err(DeviceFault::Protocol);
        }
        match self.backend.transmit(&frame[..frame_len]) {
            Ok(()) => {
                self.counters.tx_ok = self.counters.tx_ok.saturating_add(1);
                Ok(0)
            }
            Err(_) => Ok(self.drop_tx()),
        }
    }
}

impl VirtioDevice for NetDevice {
    fn device_id(&self) -> u32 {
        VIRTIO_NET_DEVICE_ID
    }

    fn feature_allowlist(&self) -> u64 {
        NET_FEATURES
    }

    fn queue_max_sizes(&self) -> &[u16] {
        &NET_QUEUE_MAX
    }

    fn config_len(&self) -> usize {
        NET_CONFIG_LEN
    }

    fn read_config(&self, offset: usize, buf: &mut [u8]) -> Result<(), ConfigAccessError> {
        let mut raw = [0u8; NET_CONFIG_LEN];
        raw[..6].copy_from_slice(&self.mac);
        let end = offset
            .checked_add(buf.len())
            .filter(|end| *end <= NET_CONFIG_LEN)
            .ok_or(ConfigAccessError::OutOfBounds)?;
        buf.copy_from_slice(&raw[offset..end]);
        Ok(())
    }

    fn write_config(&mut self, _offset: usize, _data: &[u8]) -> Result<(), ConfigAccessError> {
        Err(ConfigAccessError::ReadOnly)
    }

    fn activate(&mut self, negotiated_features: u64) -> Result<(), ActivateError> {
        if negotiated_features & !NET_FEATURES != 0 {
            return Err(ActivateError::UnsupportedFeatures {
                negotiated: negotiated_features,
            });
        }
        self.activated = true;
        Ok(())
    }

    fn reset(&mut self) {
        self.activated = false;
    }

    fn snapshot_state(&self) -> Vec<u8> {
        state::NetState::capture(self).to_bytes().to_vec()
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), DeviceStateError> {
        state::NetState::from_bytes(bytes)?.apply(self)
    }
}

#[cfg(test)]
mod hostile_tests;
#[cfg(test)]
mod tests;
