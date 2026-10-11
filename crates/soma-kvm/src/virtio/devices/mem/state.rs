//! Fixed little-endian snapshot record for the virtio-mem device model.
//!
//! It carries the geometry, the requested size, and the plugged bitmap. The
//! record length is fixed by the geometry, which is validated before the
//! bitmap is decoded, so a hostile record cannot force a large allocation.
//! Restoring the record only reinstates device-side state; making the host
//! mapping match the bitmap is the restorer's job.

use super::bitmap::PluggedBitmap;
use super::{MemDevice, MemGeometry};
use crate::virtio::device::DeviceStateError;
use crate::virtio::queue::state::le_u64;

/// Record format version.
pub const MEM_STATE_VERSION: u8 = 1;
/// Length of the fixed header before the bitmap bytes.
pub const MEM_STATE_HEADER_LEN: usize = 1 + 8 + 8 + 8 + 8 + 2;

/// Snapshot-visible virtio-mem state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemState {
    pub geometry: MemGeometry,
    pub requested_size: u64,
    pub bitmap: PluggedBitmap,
}

impl MemState {
    /// Captures a live device.
    #[must_use]
    pub fn capture(device: &MemDevice) -> Self {
        Self {
            geometry: device.geometry,
            requested_size: device.requested_size,
            bitmap: device.bitmap.clone(),
        }
    }

    /// Encodes the record.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let bitmap = self.bitmap.to_bytes();
        let mut raw = Vec::with_capacity(MEM_STATE_HEADER_LEN + bitmap.len());
        raw.push(MEM_STATE_VERSION);
        raw.extend_from_slice(&self.geometry.block_size().to_le_bytes());
        raw.extend_from_slice(&self.geometry.addr().to_le_bytes());
        raw.extend_from_slice(&self.geometry.region_size().to_le_bytes());
        raw.extend_from_slice(&self.requested_size.to_le_bytes());
        raw.extend_from_slice(&self.geometry.node_id().to_le_bytes());
        raw.extend_from_slice(&bitmap);
        raw
    }

    /// Decodes with version, geometry, exact-length, and bitmap checks.
    ///
    /// # Errors
    /// Rejects any malformed or inconsistent record.
    pub fn from_bytes(raw: &[u8]) -> Result<Self, DeviceStateError> {
        if raw.len() < MEM_STATE_HEADER_LEN || raw[0] != MEM_STATE_VERSION {
            return Err(DeviceStateError::Malformed);
        }
        let node_id = u16::from_le_bytes([raw[33], raw[34]]);
        let geometry = MemGeometry::new(
            le_u64(&raw[1..9]),
            le_u64(&raw[9..17]),
            le_u64(&raw[17..25]),
            node_id,
        )
        .map_err(|_| DeviceStateError::Malformed)?;
        let requested_size = le_u64(&raw[25..33]);
        if !requested_size.is_multiple_of(geometry.block_size())
            || requested_size > geometry.region_size()
        {
            return Err(DeviceStateError::Malformed);
        }
        let bitmap = PluggedBitmap::from_bytes(geometry.blocks(), &raw[MEM_STATE_HEADER_LEN..])
            .map_err(|_| DeviceStateError::Malformed)?;
        Ok(Self {
            geometry,
            requested_size,
            bitmap,
        })
    }

    /// Installs the record into `device`.
    ///
    /// # Errors
    /// Rejects a record whose geometry differs from the device's.
    pub fn apply(self, device: &mut MemDevice) -> Result<(), DeviceStateError> {
        if self.geometry != device.geometry {
            return Err(DeviceStateError::Incompatible);
        }
        device.requested_size = self.requested_size;
        device.bitmap = self.bitmap;
        Ok(())
    }
}
