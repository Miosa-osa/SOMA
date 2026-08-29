//! The seam between the common virtio-mmio transport and one device model.

use std::fmt;

/// Common feature bit every modern device must offer and the driver must accept.
pub const VIRTIO_F_VERSION_1: u64 = 1 << 32;
/// Vendor identifier reported by every SOMA transport (`"SOMA"` as ASCII).
pub const SOMA_VENDOR_ID: u32 = 0x534f_4d41;
/// Largest device-specific configuration space that fits one 4 KiB page.
pub const MAX_CONFIG_LEN: usize = 0x1000 - 0x100;
/// Largest queue count any v1 device declares.
pub const MAX_QUEUES: usize = 8;

/// A rejected configuration-space access.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigAccessError {
    /// `offset + len` exceeds the device configuration length.
    OutOfBounds,
    /// The field is read-only for the driver.
    ReadOnly,
    /// The access width is not permitted at this offset.
    Unaligned,
}

impl fmt::Display for ConfigAccessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "configuration access rejected: {self:?}")
    }
}

impl std::error::Error for ConfigAccessError {}

/// A rejected device activation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivateError {
    /// The negotiated feature word is not acceptable to the device model.
    UnsupportedFeatures { negotiated: u64 },
    /// The device backend is not attachable.
    BackendUnavailable,
}

impl fmt::Display for ActivateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "device activation rejected: {self:?}")
    }
}

impl std::error::Error for ActivateError {}

/// A rejected device-specific snapshot record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceStateError {
    /// The record length or layout is wrong.
    Malformed,
    /// The record is well-formed but describes an unsupported device state.
    Incompatible,
}

impl fmt::Display for DeviceStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "device state rejected: {self:?}")
    }
}

impl std::error::Error for DeviceStateError {}

/// One virtio device model behind a transport.
///
/// The transport owns the queues, status, features, and interrupt bits; the
/// device owns only its configuration space, feature allowlist, queue limits,
/// and device-specific state.
pub trait VirtioDevice {
    /// Virtio device identifier reported at `DeviceID`.
    fn device_id(&self) -> u32;

    /// Vendor identifier reported at `VendorID`.
    fn vendor_id(&self) -> u32 {
        SOMA_VENDOR_ID
    }

    /// Exact feature bits the device offers; must include [`VIRTIO_F_VERSION_1`].
    fn feature_allowlist(&self) -> u64;

    /// Fixed queue count and per-queue maximum sizes.
    fn queue_max_sizes(&self) -> &[u16];

    /// Length of the device-specific configuration space.
    fn config_len(&self) -> usize;

    /// Copies configuration bytes; `offset + buf.len()` is already bounded.
    ///
    /// # Errors
    /// Returns the typed rejection.
    fn read_config(&self, offset: usize, buf: &mut [u8]) -> Result<(), ConfigAccessError>;

    /// Applies a driver write to configuration space.
    ///
    /// # Errors
    /// Returns the typed rejection.
    fn write_config(&mut self, offset: usize, data: &[u8]) -> Result<(), ConfigAccessError>;

    /// Called once when the driver sets `DRIVER_OK` with valid queues.
    ///
    /// # Errors
    /// Returns the typed rejection; the transport then sets `DEVICE_NEEDS_RESET`.
    fn activate(&mut self, negotiated_features: u64) -> Result<(), ActivateError>;

    /// Returns the device to its unactivated state.
    fn reset(&mut self);

    /// Device-specific snapshot bytes; must exclude host handles and buffers.
    fn snapshot_state(&self) -> Vec<u8>;

    /// Restores device-specific state from a captured record.
    ///
    /// # Errors
    /// Returns the typed rejection.
    fn restore_state(&mut self, bytes: &[u8]) -> Result<(), DeviceStateError>;
}

#[cfg(test)]
pub(crate) mod test_device;
