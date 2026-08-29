//! virtio-blk (device id 2): the immutable root and private overlay models.
//!
//! One request queue, a fixed feature allowlist per role, a 24-byte
//! configuration space, and a parser that turns each chain into one
//! validated [`BlockOp`] before any backend call.

pub mod backend;
mod execute;
pub mod request;
pub mod state;

use std::fmt;

use crate::virtio::device::{
    ActivateError, ConfigAccessError, DeviceStateError, VIRTIO_F_VERSION_1, VirtioDevice,
};
use crate::virtio::devices::segments::write_writable;
use crate::virtio::devices::service::{ChainHandler, DeviceFault};
use crate::virtio::guest_memory::GuestMemory;
use crate::virtio::queue::chain::{ChainLimits, DescriptorChain};
use backend::BlockBackend;
use request::{
    MAX_REQUEST_BYTES, REQUEST_HEADER_LEN, RequestError, RequestLimits, SECTOR_SIZE,
    VIRTIO_BLK_S_IOERR, parse_request,
};

/// Virtio device identifier for a block device.
pub const VIRTIO_BLK_DEVICE_ID: u32 = 2;
/// Feature: the device is read-only.
pub const VIRTIO_BLK_F_RO: u64 = 1 << 5;
/// Feature: `blk_size` in configuration space is valid.
pub const VIRTIO_BLK_F_BLK_SIZE: u64 = 1 << 6;
/// Feature: the flush command is supported.
pub const VIRTIO_BLK_F_FLUSH: u64 = 1 << 9;
/// Queue count and maximum sizes from the device-surface table.
pub const BLOCK_QUEUE_MAX: [u16; 1] = [256];
/// Configuration space: capacity, `size_max`, `seg_max`, geometry, `blk_size`.
pub const BLOCK_CONFIG_LEN: usize = 24;
/// Device identity length reported by `GET_ID`.
pub const BLOCK_SERIAL_LEN: usize = request::BLK_ID_LEN;
const CHAIN_LIMITS: ChainLimits = ChainLimits {
    max_descriptors: 256,
    max_bytes: MAX_REQUEST_BYTES + REQUEST_HEADER_LEN + 1,
};

/// Which of the two v1 block devices this model is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockRole {
    /// Slot 0: read-only EROFS root.
    ImmutableRoot,
    /// Slot 1: private writable ext4 overlay.
    PrivateOverlay,
}

impl BlockRole {
    /// Exact feature allowlist for the role.
    #[must_use]
    pub const fn features(self) -> u64 {
        match self {
            Self::ImmutableRoot => VIRTIO_F_VERSION_1 | VIRTIO_BLK_F_RO | VIRTIO_BLK_F_BLK_SIZE,
            Self::PrivateOverlay => VIRTIO_F_VERSION_1 | VIRTIO_BLK_F_BLK_SIZE | VIRTIO_BLK_F_FLUSH,
        }
    }

    const fn read_only(self) -> bool {
        matches!(self, Self::ImmutableRoot)
    }
}

/// Why a block device could not be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockConfigError {
    /// The backend's writability disagrees with the role.
    RoleMismatch,
    /// The logical block size is not a power of two in `512..=4096`.
    InvalidBlockSize { blk_size: u32 },
    /// The capacity is zero or not a multiple of the block size.
    InvalidCapacity { capacity_bytes: u64 },
}

impl fmt::Display for BlockConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "block device rejected: {self:?}")
    }
}

impl std::error::Error for BlockConfigError {}

/// Saturating counters; never guest bytes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BlockCounters {
    pub ok: u32,
    pub io_error: u32,
    pub unsupported: u32,
    pub malformed: u32,
}

/// One block device model.
pub struct BlockDevice {
    role: BlockRole,
    backend: Box<dyn BlockBackend + Send>,
    blk_size: u32,
    capacity_sectors: u64,
    serial: [u8; BLOCK_SERIAL_LEN],
    activated: bool,
    counters: BlockCounters,
}

impl BlockDevice {
    /// Binds a backend to a role.
    ///
    /// # Errors
    /// Rejects a role/backend mismatch, a bad block size, or a bad capacity.
    pub fn new(
        role: BlockRole,
        backend: Box<dyn BlockBackend + Send>,
        blk_size: u32,
        serial: [u8; BLOCK_SERIAL_LEN],
    ) -> Result<Self, BlockConfigError> {
        if backend.read_only() != role.read_only() {
            return Err(BlockConfigError::RoleMismatch);
        }
        if !blk_size.is_power_of_two() || !(512..=4096).contains(&blk_size) {
            return Err(BlockConfigError::InvalidBlockSize { blk_size });
        }
        let capacity_bytes = backend.capacity_bytes();
        if capacity_bytes == 0 || !capacity_bytes.is_multiple_of(u64::from(blk_size)) {
            return Err(BlockConfigError::InvalidCapacity { capacity_bytes });
        }
        Ok(Self {
            role,
            backend,
            blk_size,
            capacity_sectors: capacity_bytes / SECTOR_SIZE,
            serial,
            activated: false,
            counters: BlockCounters::default(),
        })
    }

    #[must_use]
    pub const fn role(&self) -> BlockRole {
        self.role
    }

    #[must_use]
    pub const fn capacity_sectors(&self) -> u64 {
        self.capacity_sectors
    }

    #[must_use]
    pub const fn blk_size(&self) -> u32 {
        self.blk_size
    }

    #[must_use]
    pub const fn counters(&self) -> BlockCounters {
        self.counters
    }

    #[must_use]
    pub const fn is_activated(&self) -> bool {
        self.activated
    }

    fn config_bytes(&self) -> [u8; BLOCK_CONFIG_LEN] {
        let mut raw = [0u8; BLOCK_CONFIG_LEN];
        raw[0..8].copy_from_slice(&self.capacity_sectors.to_le_bytes());
        raw[20..24].copy_from_slice(&self.blk_size.to_le_bytes());
        raw
    }

    fn limits(&self) -> RequestLimits {
        RequestLimits {
            capacity_bytes: self.capacity_sectors * SECTOR_SIZE,
            read_only: self.role.read_only(),
            flush: self.role.features() & VIRTIO_BLK_F_FLUSH != 0,
        }
    }
}

impl ChainHandler for BlockDevice {
    fn chain_limits(&self, _queue: u16) -> ChainLimits {
        CHAIN_LIMITS
    }

    fn handle_chain<M: GuestMemory + ?Sized>(
        &mut self,
        _queue: u16,
        chain: &DescriptorChain,
        mem: &M,
    ) -> Result<u32, DeviceFault> {
        let (status, data_len, status_skip, error) = match parse_request(mem, chain, self.limits())
        {
            Ok(request) => {
                let (status, data_len) = self.execute(request, chain, mem)?;
                (status, data_len, request.status_skip, None)
            }
            Err(RequestError::NoStatus) => {
                self.count(VIRTIO_BLK_S_IOERR, Some(RequestError::NoStatus));
                return Ok(0);
            }
            Err(error) => (
                error.status(),
                0,
                chain.writable_len().saturating_sub(1),
                Some(error),
            ),
        };
        self.count(status, error);
        if write_writable(mem, chain, status_skip, &[status])? != 1 {
            return Err(DeviceFault::Protocol);
        }
        Ok(data_len.saturating_add(1))
    }
}

impl VirtioDevice for BlockDevice {
    fn device_id(&self) -> u32 {
        VIRTIO_BLK_DEVICE_ID
    }

    fn feature_allowlist(&self) -> u64 {
        self.role.features()
    }

    fn queue_max_sizes(&self) -> &[u16] {
        &BLOCK_QUEUE_MAX
    }

    fn config_len(&self) -> usize {
        BLOCK_CONFIG_LEN
    }

    fn read_config(&self, offset: usize, buf: &mut [u8]) -> Result<(), ConfigAccessError> {
        let raw = self.config_bytes();
        let end = offset
            .checked_add(buf.len())
            .filter(|end| *end <= BLOCK_CONFIG_LEN)
            .ok_or(ConfigAccessError::OutOfBounds)?;
        buf.copy_from_slice(&raw[offset..end]);
        Ok(())
    }

    fn write_config(&mut self, _offset: usize, _data: &[u8]) -> Result<(), ConfigAccessError> {
        Err(ConfigAccessError::ReadOnly)
    }

    fn activate(&mut self, negotiated_features: u64) -> Result<(), ActivateError> {
        if negotiated_features & !self.role.features() != 0 {
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
        state::BlockState::capture(self).to_bytes().to_vec()
    }

    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), DeviceStateError> {
        state::BlockState::from_bytes(bytes)?.apply(self)
    }
}

#[cfg(test)]
mod hostile_tests;
#[cfg(test)]
mod identity_tests;
#[cfg(test)]
mod tests;
