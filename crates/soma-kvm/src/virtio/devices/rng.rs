//! virtio-rng (device id 4): one request queue filled from a fresh host
//! CSPRNG, bounded per request, with no random byte ever retained.

pub mod backend;
pub mod state;

use crate::virtio::device::{
    ActivateError, ConfigAccessError, DeviceStateError, VIRTIO_F_VERSION_1, VirtioDevice,
};
use crate::virtio::devices::segments::write_writable;
use crate::virtio::devices::service::{ChainHandler, DeviceFault};
use crate::virtio::guest_memory::GuestMemory;
use crate::virtio::queue::chain::{ChainLimits, DescriptorChain};
use backend::EntropyBackend;

/// Virtio device identifier for an entropy device.
pub const VIRTIO_RNG_DEVICE_ID: u32 = 4;
/// Exact feature allowlist: the common modern bit only.
pub const RNG_FEATURES: u64 = VIRTIO_F_VERSION_1;
/// Queue count and maximum size from the device-surface table.
pub const RNG_QUEUE_MAX: [u16; 1] = [64];
/// Largest number of bytes one request receives; longer chains are capped.
pub const MAX_ENTROPY_REQUEST: usize = 1 << 16;
const CHAIN_LIMITS: ChainLimits = ChainLimits {
    max_descriptors: 64,
    max_bytes: 1 << 20,
};

/// Saturating counters; never random bytes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RngCounters {
    pub filled: u32,
    pub bytes: u64,
    pub rejected: u32,
}

/// The entropy device model.
pub struct RngDevice {
    backend: Box<dyn EntropyBackend + Send>,
    activated: bool,
    counters: RngCounters,
}

impl RngDevice {
    /// Binds a fresh entropy source.
    #[must_use]
    pub fn new(backend: Box<dyn EntropyBackend + Send>) -> Self {
        Self {
            backend,
            activated: false,
            counters: RngCounters::default(),
        }
    }

    #[must_use]
    pub const fn counters(&self) -> RngCounters {
        self.counters
    }

    #[must_use]
    pub const fn is_activated(&self) -> bool {
        self.activated
    }
}

impl ChainHandler for RngDevice {
    fn chain_limits(&self, _queue: u16) -> ChainLimits {
        CHAIN_LIMITS
    }

    fn handle_chain<M: GuestMemory + ?Sized>(
        &mut self,
        _queue: u16,
        chain: &DescriptorChain,
        mem: &M,
    ) -> Result<u32, DeviceFault> {
        if chain.readable_len() != 0 {
            self.counters.rejected = self.counters.rejected.saturating_add(1);
            return Ok(0);
        }
        let max = u64::try_from(MAX_ENTROPY_REQUEST).unwrap_or(u64::MAX);
        let len = chain.writable_len().min(max);
        let len = usize::try_from(len).map_err(|_| DeviceFault::Protocol)?;
        let mut bytes = vec![0u8; len];
        self.backend
            .fill(&mut bytes)
            .map_err(|_| DeviceFault::Backend)?;
        if write_writable(mem, chain, 0, &bytes)? != len {
            return Err(DeviceFault::Protocol);
        }
        self.counters.filled = self.counters.filled.saturating_add(1);
        self.counters.bytes = self
            .counters
            .bytes
            .saturating_add(u64::try_from(len).unwrap_or(u64::MAX));
        u32::try_from(len).map_err(|_| DeviceFault::Protocol)
    }
}

impl VirtioDevice for RngDevice {
    fn device_id(&self) -> u32 {
        VIRTIO_RNG_DEVICE_ID
    }

    fn feature_allowlist(&self) -> u64 {
        RNG_FEATURES
    }

    fn queue_max_sizes(&self) -> &[u16] {
        &RNG_QUEUE_MAX
    }

    fn config_len(&self) -> usize {
        0
    }

    fn read_config(&self, _offset: usize, _buf: &mut [u8]) -> Result<(), ConfigAccessError> {
        Err(ConfigAccessError::OutOfBounds)
    }

    fn write_config(&mut self, _offset: usize, _data: &[u8]) -> Result<(), ConfigAccessError> {
        Err(ConfigAccessError::OutOfBounds)
    }

    fn activate(&mut self, negotiated_features: u64) -> Result<(), ActivateError> {
        if negotiated_features & !RNG_FEATURES != 0 {
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
        state::RngState::capture().to_bytes().to_vec()
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), DeviceStateError> {
        state::RngState::from_bytes(bytes)?.apply()
    }
}

#[cfg(test)]
mod tests;
