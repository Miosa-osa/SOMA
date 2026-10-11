//! The host-side effect seam for plug and unplug.
//!
//! The device decides whether a request is valid; the backend makes the host
//! mapping match. The device updates its bitmap only after the backend
//! reports success, so a refused or failed operation leaves both unchanged.

use std::fmt;

/// Why the host could not apply a plug or unplug.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemBackendError {
    /// The host is momentarily unable; the device answers `BUSY`.
    Busy,
    /// The host operation failed; the device answers `ERROR`.
    Failed,
}

impl fmt::Display for MemBackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "memory backend refused: {self:?}")
    }
}

impl std::error::Error for MemBackendError {}

/// Makes the host mapping of the device region follow plug state.
///
/// Every `addr` is a guest-physical address inside the device region and every
/// range is a whole number of device blocks; the device has already validated
/// both.
pub trait MemBackend {
    /// Makes `[addr, addr + len)` backed and accessible.
    ///
    /// # Errors
    /// Returns the typed refusal; plug state is then unchanged.
    fn plug(&mut self, addr: u64, len: u64) -> Result<(), MemBackendError>;

    /// Releases the host backing of `[addr, addr + len)`.
    ///
    /// # Errors
    /// Returns the typed refusal; plug state is then unchanged.
    fn unplug(&mut self, addr: u64, len: u64) -> Result<(), MemBackendError>;

    /// Releases the host backing of the whole device region.
    ///
    /// # Errors
    /// Returns the typed refusal; plug state is then unchanged.
    fn unplug_all(&mut self) -> Result<(), MemBackendError>;
}

/// A backend with no host effect, for tests and for hosts that track plug
/// state elsewhere.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullBackend;

impl MemBackend for NullBackend {
    fn plug(&mut self, _addr: u64, _len: u64) -> Result<(), MemBackendError> {
        Ok(())
    }

    fn unplug(&mut self, _addr: u64, _len: u64) -> Result<(), MemBackendError> {
        Ok(())
    }

    fn unplug_all(&mut self) -> Result<(), MemBackendError> {
        Ok(())
    }
}
