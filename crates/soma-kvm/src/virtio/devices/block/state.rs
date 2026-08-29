//! Fixed little-endian snapshot record for one block device model.
//!
//! The record carries identity only: version, device id, feature allowlist,
//! role, block size, capacity, and serial. Backend handles, paths, and data
//! never enter it, and restore requires exact equality with the live device.

use super::{BLOCK_SERIAL_LEN, BlockDevice, BlockRole, VIRTIO_BLK_DEVICE_ID};
use crate::virtio::device::DeviceStateError;
use crate::virtio::queue::state::le_u64;

/// Record format version.
pub const BLOCK_STATE_VERSION: u8 = 1;
/// Encoded length.
pub const BLOCK_STATE_LEN: usize = 1 + 4 + 8 + 1 + 4 + 8 + BLOCK_SERIAL_LEN;

/// Snapshot-visible block device identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockState {
    pub device_id: u32,
    pub features: u64,
    pub role: BlockRole,
    pub blk_size: u32,
    pub capacity_sectors: u64,
    pub serial: [u8; BLOCK_SERIAL_LEN],
}

impl BlockState {
    /// Captures the identity of a live device.
    #[must_use]
    pub fn capture(device: &BlockDevice) -> Self {
        Self {
            device_id: VIRTIO_BLK_DEVICE_ID,
            features: device.role.features(),
            role: device.role,
            blk_size: device.blk_size,
            capacity_sectors: device.capacity_sectors,
            serial: device.serial,
        }
    }

    /// Encodes the record.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; BLOCK_STATE_LEN] {
        let mut raw = [0u8; BLOCK_STATE_LEN];
        raw[0] = BLOCK_STATE_VERSION;
        raw[1..5].copy_from_slice(&self.device_id.to_le_bytes());
        raw[5..13].copy_from_slice(&self.features.to_le_bytes());
        raw[13] = match self.role {
            BlockRole::ImmutableRoot => 0,
            BlockRole::PrivateOverlay => 1,
        };
        raw[14..18].copy_from_slice(&self.blk_size.to_le_bytes());
        raw[18..26].copy_from_slice(&self.capacity_sectors.to_le_bytes());
        raw[26..].copy_from_slice(&self.serial);
        raw
    }

    /// Decodes with exact-length and version checks.
    ///
    /// # Errors
    /// Rejects a wrong length, version, or role byte.
    pub fn from_bytes(raw: &[u8]) -> Result<Self, DeviceStateError> {
        if raw.len() != BLOCK_STATE_LEN || raw[0] != BLOCK_STATE_VERSION {
            return Err(DeviceStateError::Malformed);
        }
        let role = match raw[13] {
            0 => BlockRole::ImmutableRoot,
            1 => BlockRole::PrivateOverlay,
            _ => return Err(DeviceStateError::Malformed),
        };
        let mut serial = [0u8; BLOCK_SERIAL_LEN];
        serial.copy_from_slice(&raw[26..]);
        Ok(Self {
            device_id: u32::try_from(le_u64(&raw[1..5]))
                .map_err(|_| DeviceStateError::Malformed)?,
            features: le_u64(&raw[5..13]),
            role,
            blk_size: u32::try_from(le_u64(&raw[14..18]))
                .map_err(|_| DeviceStateError::Malformed)?,
            capacity_sectors: le_u64(&raw[18..26]),
            serial,
        })
    }

    /// Verifies the record against a live device; nothing is mutated because
    /// every field is fixed by the Generation, not by the guest.
    ///
    /// # Errors
    /// Rejects any mismatch as incompatible.
    pub fn apply(self, device: &BlockDevice) -> Result<(), DeviceStateError> {
        if self == Self::capture(device) {
            Ok(())
        } else {
            Err(DeviceStateError::Incompatible)
        }
    }
}
