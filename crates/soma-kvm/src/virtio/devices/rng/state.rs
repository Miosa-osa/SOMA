//! Fixed little-endian snapshot record for the entropy device model.
//!
//! It carries identity only: version, device id, and feature allowlist.
//! No random byte, source handle, or generator state is ever encoded.

use super::{RNG_FEATURES, VIRTIO_RNG_DEVICE_ID};
use crate::virtio::device::DeviceStateError;
use crate::virtio::queue::state::le_u64;

/// Record format version.
pub const RNG_STATE_VERSION: u8 = 1;
/// Encoded length.
pub const RNG_STATE_LEN: usize = 1 + 4 + 8;

/// Snapshot-visible entropy device identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RngState {
    pub device_id: u32,
    pub features: u64,
}

impl RngState {
    /// The identity of every entropy device.
    #[must_use]
    pub const fn capture() -> Self {
        Self {
            device_id: VIRTIO_RNG_DEVICE_ID,
            features: RNG_FEATURES,
        }
    }

    /// Encodes the record.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; RNG_STATE_LEN] {
        let mut raw = [0u8; RNG_STATE_LEN];
        raw[0] = RNG_STATE_VERSION;
        raw[1..5].copy_from_slice(&self.device_id.to_le_bytes());
        raw[5..13].copy_from_slice(&self.features.to_le_bytes());
        raw
    }

    /// Decodes with exact-length and version checks.
    ///
    /// # Errors
    /// Rejects a wrong length or version.
    pub fn from_bytes(raw: &[u8]) -> Result<Self, DeviceStateError> {
        if raw.len() != RNG_STATE_LEN || raw[0] != RNG_STATE_VERSION {
            return Err(DeviceStateError::Malformed);
        }
        Ok(Self {
            device_id: u32::try_from(le_u64(&raw[1..5]))
                .map_err(|_| DeviceStateError::Malformed)?,
            features: le_u64(&raw[5..13]),
        })
    }

    /// Verifies identity; there is nothing to install.
    ///
    /// # Errors
    /// Rejects any identity mismatch.
    pub fn apply(self) -> Result<(), DeviceStateError> {
        if self == Self::capture() {
            Ok(())
        } else {
            Err(DeviceStateError::Incompatible)
        }
    }
}
