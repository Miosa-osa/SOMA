//! Fixed little-endian snapshot record for the network device model.
//!
//! It carries the version, device id, feature allowlist, placeholder MAC,
//! and link state. It never carries a TAP descriptor, interface name, or
//! frame, and a capture with the link up is refused on restore.

use super::{NetDevice, VIRTIO_NET_DEVICE_ID};
use crate::virtio::device::DeviceStateError;
use crate::virtio::queue::state::le_u64;

/// Record format version.
pub const NET_STATE_VERSION: u8 = 1;
/// Encoded length.
pub const NET_STATE_LEN: usize = 1 + 4 + 8 + 6 + 1;

/// Snapshot-visible network device state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetState {
    pub device_id: u32,
    pub features: u64,
    pub mac: [u8; 6],
    pub link_up: bool,
}

impl NetState {
    /// Captures a live device.
    #[must_use]
    pub fn capture(device: &NetDevice) -> Self {
        Self {
            device_id: VIRTIO_NET_DEVICE_ID,
            features: NetDevice::feature_allowlist_value(),
            mac: device.mac(),
            link_up: device.link_up(),
        }
    }

    /// Encodes the record.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; NET_STATE_LEN] {
        let mut raw = [0u8; NET_STATE_LEN];
        raw[0] = NET_STATE_VERSION;
        raw[1..5].copy_from_slice(&self.device_id.to_le_bytes());
        raw[5..13].copy_from_slice(&self.features.to_le_bytes());
        raw[13..19].copy_from_slice(&self.mac);
        raw[19] = u8::from(self.link_up);
        raw
    }

    /// Decodes with exact-length, version, and flag checks.
    ///
    /// # Errors
    /// Rejects a wrong length, version, or flag byte.
    pub fn from_bytes(raw: &[u8]) -> Result<Self, DeviceStateError> {
        if raw.len() != NET_STATE_LEN || raw[0] != NET_STATE_VERSION {
            return Err(DeviceStateError::Malformed);
        }
        let link_up = match raw[19] {
            0 => false,
            1 => true,
            _ => return Err(DeviceStateError::Malformed),
        };
        let mut mac = [0u8; 6];
        mac.copy_from_slice(&raw[13..19]);
        Ok(Self {
            device_id: u32::try_from(le_u64(&raw[1..5]))
                .map_err(|_| DeviceStateError::Malformed)?,
            features: le_u64(&raw[5..13]),
            mac,
            link_up,
        })
    }

    /// Installs the captured placeholder MAC; identity must match and the
    /// captured link must be down because restore never raises it.
    ///
    /// # Errors
    /// Rejects any identity mismatch or a captured link-up state.
    pub fn apply(self, device: &mut NetDevice) -> Result<(), DeviceStateError> {
        if self.device_id != VIRTIO_NET_DEVICE_ID
            || self.features != NetDevice::feature_allowlist_value()
            || self.link_up
        {
            return Err(DeviceStateError::Incompatible);
        }
        device.set_mac(self.mac);
        device.set_link(false);
        Ok(())
    }
}
