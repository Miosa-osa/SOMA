//! Fixed little-endian snapshot record for the memory device model.
//!
//! Only the region geometry is recorded. Plugged memory is not part of a snapshot, so the record
//! has no bitmap and no requested size, a capture is refused while anything is plugged, and
//! restore always yields an empty region.

use super::{BLOCK_SIZE, MEM_FEATURES, MemDevice, VIRTIO_MEM_DEVICE_ID};
use crate::virtio::device::DeviceStateError;
use crate::virtio::queue::state::le_u64;

/// Record format version.
pub const MEM_STATE_VERSION: u8 = 1;
/// Encoded length.
pub const MEM_STATE_LEN: usize = 1 + 4 + 8 + 8 + 8 + 8;

/// Snapshot-visible memory device state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemState {
    pub device_id: u32,
    pub features: u64,
    pub addr: u64,
    pub region_size: u64,
    pub block_size: u64,
}

impl MemState {
    /// Captures the geometry of a live device.
    #[must_use]
    pub const fn capture(device: &MemDevice) -> Self {
        Self {
            device_id: VIRTIO_MEM_DEVICE_ID,
            features: MEM_FEATURES,
            addr: device.addr,
            region_size: device.region_size,
            block_size: BLOCK_SIZE,
        }
    }

    /// Encodes the record.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; MEM_STATE_LEN] {
        let mut raw = [0_u8; MEM_STATE_LEN];
        raw[0] = MEM_STATE_VERSION;
        raw[1..5].copy_from_slice(&self.device_id.to_le_bytes());
        raw[5..13].copy_from_slice(&self.features.to_le_bytes());
        raw[13..21].copy_from_slice(&self.addr.to_le_bytes());
        raw[21..29].copy_from_slice(&self.region_size.to_le_bytes());
        raw[29..37].copy_from_slice(&self.block_size.to_le_bytes());
        raw
    }

    /// Decodes with exact-length and version checks.
    ///
    /// # Errors
    /// Rejects a wrong length or version.
    pub fn from_bytes(raw: &[u8]) -> Result<Self, DeviceStateError> {
        if raw.len() != MEM_STATE_LEN || raw[0] != MEM_STATE_VERSION {
            return Err(DeviceStateError::Malformed);
        }
        Ok(Self {
            device_id: u32::try_from(le_u64(&raw[1..5]))
                .map_err(|_| DeviceStateError::Malformed)?,
            features: le_u64(&raw[5..13]),
            addr: le_u64(&raw[13..21]),
            region_size: le_u64(&raw[21..29]),
            block_size: le_u64(&raw[29..37]),
        })
    }

    /// Checks the record describes this device's region and leaves the region empty.
    ///
    /// # Errors
    /// Rejects an identity or geometry mismatch.
    pub fn apply(self, device: &mut MemDevice) -> Result<(), DeviceStateError> {
        let same = self.device_id == VIRTIO_MEM_DEVICE_ID
            && self.features == MEM_FEATURES
            && self.addr == device.addr
            && self.region_size == device.region_size
            && self.block_size == BLOCK_SIZE;
        if !same {
            return Err(DeviceStateError::Incompatible);
        }
        device.bitmap.fill(0);
        device.plugged_blocks = 0;
        device.requested = 0;
        Ok(())
    }
}
