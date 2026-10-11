//! virtio-mem (device id 24): memory that is added to and taken from a running guest.
//!
//! The machine boots with exactly the memory it was paid for. Beside it the device describes an
//! empty region, up to the contract ceiling, that the guest driver fills in 2 MiB blocks when the
//! host raises `requested_size` and empties when the host lowers it. The guest's `MemTotal` is
//! the proof: it grows by exactly the blocks the device confirmed and no more.
//!
//! The device validates every request against the region and against `requested_size`, so a
//! guest cannot take more than it was given, cannot plug a block twice, and cannot unplug one it
//! never held. An unplugged block is handed back to the host through [`RegionBacking`], so a
//! shrink really frees memory.
//!
//! Plugged memory is not in a snapshot. The record carries the region geometry only, a capture is
//! refused while anything is plugged, and a restore always starts empty.

pub mod backing;
mod requests;
pub mod state;
mod types;

#[cfg(test)]
mod atomicity_tests;
#[cfg(test)]
mod bounds_tests;
#[cfg(test)]
mod functional_tests;
#[cfg(test)]
mod hostile_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod model_tests;
#[cfg(test)]
mod race_tests;
#[cfg(test)]
mod rig;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod wire_tests;

use crate::virtio::device::{
    ActivateError, ConfigAccessError, DeviceStateError, VIRTIO_F_VERSION_1, VirtioDevice,
};
use crate::virtio::devices::segments::{read_readable, write_writable};
use crate::virtio::devices::service::{ChainHandler, DeviceFault};
use crate::virtio::guest_memory::GuestMemory;
use crate::virtio::queue::chain::{ChainLimits, DescriptorChain};
use backing::RegionBacking;
pub use types::{MemCounters, SizeError};

/// Virtio device identifier for a memory device.
pub const VIRTIO_MEM_DEVICE_ID: u32 = 24;
/// Exact feature allowlist: the common modern bit only.
pub const MEM_FEATURES: u64 = VIRTIO_F_VERSION_1;
/// One request queue.
pub const MEM_QUEUE_MAX: [u16; 1] = [16];
/// The size of one block: the unit of every plug and unplug.
pub const BLOCK_SIZE: u64 = 2 * 1024 * 1024;
/// Configuration space length.
pub const MEM_CONFIG_LEN: usize = 56;

const REQUEST_LEN: usize = 24;
const RESPONSE_LEN: usize = 10;
const REQUEST_WIRE: u32 = 24;
const RESPONSE_WIRE: u32 = 10;
/// The most blocks one request may name.
const MAX_REQUEST_BLOCKS: u64 = 1 << 12;
const CHAIN_LIMITS: ChainLimits = ChainLimits {
    max_descriptors: 8,
    max_bytes: 4096,
};

const REQ_PLUG: u16 = 0;
const REQ_UNPLUG: u16 = 1;
const REQ_UNPLUG_ALL: u16 = 2;
const REQ_STATE: u16 = 3;
const RESP_ACK: u16 = 0;
const RESP_NACK: u16 = 1;
const RESP_ERROR: u16 = 3;
const STATE_PLUGGED: u16 = 0;
const STATE_UNPLUGGED: u16 = 1;
const STATE_MIXED: u16 = 2;

/// The memory device model.
pub struct MemDevice {
    addr: u64,
    region_size: u64,
    requested: u64,
    /// One bit per block: set when the block is plugged.
    bitmap: Vec<u64>,
    plugged_blocks: u64,
    backing: Box<dyn RegionBacking + Send>,
    activated: bool,
    /// Set when a reset could not return the plugged blocks, so they are still tracked.
    stale: bool,
    counters: MemCounters,
}

impl MemDevice {
    /// A device for a region of `region_size` bytes at guest address `addr`.
    ///
    /// # Errors
    /// Rejects a region that is empty, unaligned to the block size, or too large to track.
    pub fn new(
        addr: u64,
        region_size: u64,
        backing: Box<dyn RegionBacking + Send>,
    ) -> Result<Self, SizeError> {
        if region_size == 0
            || !addr.is_multiple_of(BLOCK_SIZE)
            || !region_size.is_multiple_of(BLOCK_SIZE)
        {
            return Err(SizeError::Unaligned);
        }
        let blocks = region_size / BLOCK_SIZE;
        if blocks > 1 << 20 || addr.checked_add(region_size).is_none() {
            return Err(SizeError::BeyondRegion);
        }
        Ok(Self {
            addr,
            region_size,
            requested: 0,
            bitmap: vec![
                0;
                usize::try_from(blocks.div_ceil(64))
                    .map_err(|_| SizeError::BeyondRegion)?
            ],
            plugged_blocks: 0,
            backing,
            activated: false,
            stale: false,
            counters: MemCounters::default(),
        })
    }

    /// Asks the guest for `bytes` of plugged memory in total.
    ///
    /// The caller signals the configuration change to the driver afterwards; the guest then plugs
    /// or unplugs toward the size, and [`Self::plugged_size`] says what it actually holds.
    ///
    /// # Errors
    /// Rejects a size that is not a whole number of blocks or exceeds the region.
    pub fn set_requested_size(&mut self, bytes: u64) -> Result<(), SizeError> {
        if !bytes.is_multiple_of(BLOCK_SIZE) {
            return Err(SizeError::Unaligned);
        }
        if bytes > self.region_size {
            return Err(SizeError::BeyondRegion);
        }
        self.requested = bytes;
        Ok(())
    }

    /// The memory the guest holds on top of what it booted with, as the device has confirmed it.
    #[must_use]
    pub const fn plugged_size(&self) -> u64 {
        self.plugged_blocks * BLOCK_SIZE
    }

    /// The size the host last asked for.
    #[must_use]
    pub const fn requested_size(&self) -> u64 {
        self.requested
    }

    /// The bytes of the region.
    #[must_use]
    pub const fn region_size(&self) -> u64 {
        self.region_size
    }

    /// The guest address the region starts at.
    #[must_use]
    pub const fn region_addr(&self) -> u64 {
        self.addr
    }

    #[must_use]
    pub const fn counters(&self) -> MemCounters {
        self.counters
    }

    /// Hands every plugged block back to the host, and forgets them only if it took them.
    fn release_all(&mut self) -> bool {
        if self.backing.discard(0, self.region_size).is_err() {
            self.counters.discard_failed = self.counters.discard_failed.saturating_add(1);
            return false;
        }
        self.bitmap.fill(0);
        self.plugged_blocks = 0;
        self.stale = false;
        true
    }

    /// Whether nothing is plugged, the only state in which a capture is permitted.
    #[must_use]
    pub const fn is_quiescent(&self) -> bool {
        self.plugged_blocks == 0
    }
}

impl ChainHandler for MemDevice {
    fn chain_limits(&self, _queue: u16) -> ChainLimits {
        CHAIN_LIMITS
    }

    fn handle_chain<M: GuestMemory + ?Sized>(
        &mut self,
        _queue: u16,
        chain: &DescriptorChain,
        mem: &M,
    ) -> Result<u32, DeviceFault> {
        let well_formed = chain.readable_len() == u64::from(REQUEST_WIRE)
            && chain.writable_len() >= u64::from(RESPONSE_WIRE);
        if !well_formed {
            self.counters.rejected = self.counters.rejected.saturating_add(1);
            return Ok(0);
        }
        let mut request = [0_u8; REQUEST_LEN];
        if read_readable(mem, chain, 0, &mut request)? != REQUEST_LEN {
            return Err(DeviceFault::Protocol);
        }
        let (kind, state) = self.answer(&request);
        let mut response = [0_u8; RESPONSE_LEN];
        response[0..2].copy_from_slice(&kind.to_le_bytes());
        response[8..10].copy_from_slice(&state.to_le_bytes());
        if write_writable(mem, chain, 0, &response)? != RESPONSE_LEN {
            return Err(DeviceFault::Protocol);
        }
        Ok(RESPONSE_WIRE)
    }
}

impl VirtioDevice for MemDevice {
    fn device_id(&self) -> u32 {
        VIRTIO_MEM_DEVICE_ID
    }

    fn feature_allowlist(&self) -> u64 {
        MEM_FEATURES
    }

    fn queue_max_sizes(&self) -> &[u16] {
        &MEM_QUEUE_MAX
    }

    fn config_len(&self) -> usize {
        MEM_CONFIG_LEN
    }

    fn read_config(&self, offset: usize, buf: &mut [u8]) -> Result<(), ConfigAccessError> {
        let mut raw = [0_u8; MEM_CONFIG_LEN];
        raw[0..8].copy_from_slice(&BLOCK_SIZE.to_le_bytes());
        raw[16..24].copy_from_slice(&self.addr.to_le_bytes());
        raw[24..32].copy_from_slice(&self.region_size.to_le_bytes());
        raw[32..40].copy_from_slice(&self.region_size.to_le_bytes());
        raw[40..48].copy_from_slice(&self.plugged_size().to_le_bytes());
        raw[48..56].copy_from_slice(&self.requested.to_le_bytes());
        let end = offset
            .checked_add(buf.len())
            .filter(|end| *end <= MEM_CONFIG_LEN)
            .ok_or(ConfigAccessError::OutOfBounds)?;
        buf.copy_from_slice(&raw[offset..end]);
        Ok(())
    }

    fn write_config(&mut self, _offset: usize, _data: &[u8]) -> Result<(), ConfigAccessError> {
        Err(ConfigAccessError::ReadOnly)
    }

    fn activate(&mut self, negotiated_features: u64) -> Result<(), ActivateError> {
        if negotiated_features & !MEM_FEATURES != 0 {
            return Err(ActivateError::UnsupportedFeatures {
                negotiated: negotiated_features,
            });
        }
        // A reset that could not hand its blocks back left them tracked. A new driver starts from
        // nothing, so they are returned before it may begin; if the host still cannot, the driver
        // is refused rather than started over memory the device no longer accounts for.
        if self.stale && !self.release_all() {
            return Err(ActivateError::BackendUnavailable);
        }
        self.activated = true;
        Ok(())
    }

    fn reset(&mut self) {
        // A reset driver has forgotten every block it held. The device forgets them too, but
        // only once the host has taken the memory back: a block that could not be returned stays
        // tracked and counted, so the plugged size stays true and nothing is untracked silently.
        self.activated = false;
        if self.plugged_blocks > 0 {
            self.stale = !self.release_all();
        }
    }

    fn snapshot_state(&self) -> Vec<u8> {
        state::MemState::capture(self).to_bytes().to_vec()
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), DeviceStateError> {
        state::MemState::from_bytes(bytes)?.apply(self)
    }
}
