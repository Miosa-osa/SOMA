//! A block backend that declares a store's shape while holding no store.

use super::{BackendError, BlockBackend};

/// A backend that declares a store's shape while holding no store at all.
///
/// A prepared worker is built before any Instance exists, and the prepared worker protocol
/// forbids it from holding a private disk head.
/// The device can still be constructed from the admitted capacity and writability.
/// Every I/O operation fails closed because an unassigned worker must never run a vCPU.
#[derive(Clone, Copy, Debug)]
#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
pub struct Detached {
    capacity_bytes: u64,
    read_only: bool,
}

#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
impl Detached {
    /// Declares the shape of the store that will be attached later.
    #[must_use]
    pub const fn new(capacity_bytes: u64, read_only: bool) -> Self {
        Self {
            capacity_bytes,
            read_only,
        }
    }
}

#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
impl BlockBackend for Detached {
    fn capacity_bytes(&self) -> u64 {
        self.capacity_bytes
    }

    fn read_only(&self) -> bool {
        self.read_only
    }

    fn read_at(&mut self, _offset: u64, _buf: &mut [u8]) -> Result<usize, BackendError> {
        Err(BackendError::OutOfRange)
    }

    fn write_at(&mut self, _offset: u64, _data: &[u8]) -> Result<usize, BackendError> {
        Err(BackendError::OutOfRange)
    }

    fn flush(&mut self) -> Result<(), BackendError> {
        Err(BackendError::OutOfRange)
    }
}
