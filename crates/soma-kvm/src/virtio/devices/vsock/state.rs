//! Fixed little-endian snapshot record for the vsock device model.
//!
//! It carries the version, device id, feature allowlist, and the captured
//! CID placeholder only. No connection, credit window, buffered byte, or
//! event is encoded, and restore clears all of them and queues one
//! `TRANSPORT_RESET` event for the guest.

use super::packet::VIRTIO_VSOCK_DEVICE_ID;
use super::{VSOCK_FEATURES, VsockDevice};
use crate::virtio::device::DeviceStateError;
use crate::virtio::queue::state::le_u64;

/// Record format version.
pub const VSOCK_STATE_VERSION: u8 = 1;
/// Encoded length.
pub const VSOCK_STATE_LEN: usize = 1 + 4 + 8 + 8;

/// Snapshot-visible vsock device state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VsockState {
    pub device_id: u32,
    pub features: u64,
    pub guest_cid: u64,
}

impl VsockState {
    /// Captures a live device.
    #[must_use]
    pub const fn capture(device: &VsockDevice) -> Self {
        Self {
            device_id: VIRTIO_VSOCK_DEVICE_ID,
            features: VSOCK_FEATURES,
            guest_cid: device.guest_cid,
        }
    }

    /// Encodes the record.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; VSOCK_STATE_LEN] {
        let mut raw = [0u8; VSOCK_STATE_LEN];
        raw[0] = VSOCK_STATE_VERSION;
        raw[1..5].copy_from_slice(&self.device_id.to_le_bytes());
        raw[5..13].copy_from_slice(&self.features.to_le_bytes());
        raw[13..21].copy_from_slice(&self.guest_cid.to_le_bytes());
        raw
    }

    /// Decodes with exact-length and version checks.
    ///
    /// # Errors
    /// Rejects a wrong length or version.
    pub fn from_bytes(raw: &[u8]) -> Result<Self, DeviceStateError> {
        if raw.len() != VSOCK_STATE_LEN || raw[0] != VSOCK_STATE_VERSION {
            return Err(DeviceStateError::Malformed);
        }
        Ok(Self {
            device_id: u32::try_from(le_u64(&raw[1..5]))
                .map_err(|_| DeviceStateError::Malformed)?,
            features: le_u64(&raw[5..13]),
            guest_cid: le_u64(&raw[13..21]),
        })
    }

    /// Installs the captured CID placeholder, clears every connection and
    /// credit state, and queues the transport-reset event.
    ///
    /// # Errors
    /// Rejects an identity mismatch or a reserved CID.
    pub fn apply(self, device: &mut VsockDevice) -> Result<(), DeviceStateError> {
        if self.device_id != VIRTIO_VSOCK_DEVICE_ID || self.features != VSOCK_FEATURES {
            return Err(DeviceStateError::Incompatible);
        }
        device
            .set_guest_cid(self.guest_cid)
            .map_err(|_| DeviceStateError::Incompatible)?;
        device.after_restore();
        Ok(())
    }
}
