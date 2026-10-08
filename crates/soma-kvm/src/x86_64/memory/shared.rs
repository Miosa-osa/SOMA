//! The checked guest-physical view the device threads and the capture walk read through.
//!
//! The devices address guest memory by physical address, so every access is translated through
//! the machine layout before a byte is touched: a machine whose RAM continues above the MMIO
//! boundary keeps one object whose second range starts at a different guest address than object
//! offset, and an address inside the hole is refused rather than reading the wrong bytes.

use std::sync::Arc;

use super::RamMapping;
use crate::memory_layout::GuestLayout;
use crate::virtio::{GuestAddress, GuestMemory, GuestMemoryError};

/// The checked guest-physical view used by the virtio devices and the MMIO dispatcher.
///
/// The devices address guest memory by physical address, so every access is translated through
/// the layout first: a machine whose RAM continues above the MMIO boundary keeps one object
/// whose second range starts at a different guest address than object offset, and a device
/// naming an address inside the hole is refused rather than reading the wrong bytes.
#[derive(Clone)]
pub struct SharedRam {
    pub(super) mapping: Arc<RamMapping>,
    pub(super) layout: GuestLayout,
}

impl SharedRam {
    /// Copies `buf.len()` bytes of the memory object starting at object offset `offset`.
    ///
    /// This is the capture walk's view: the walk reads the whole object linearly, across both
    /// ranges of a split machine, so it names object offsets rather than guest addresses.
    pub(crate) fn read_image(&self, offset: u64, buf: &mut [u8]) -> bool {
        self.mapping.read(offset, buf)
    }
}

impl GuestMemory for SharedRam {
    fn check_range(&self, addr: GuestAddress, len: u64) -> Result<(), GuestMemoryError> {
        if len == 0 {
            return Ok(());
        }
        addr.checked_add(len)
            .ok_or(GuestMemoryError::Overflow { addr, len })?;
        if self.layout.contains(addr.raw(), len) {
            Ok(())
        } else {
            Err(GuestMemoryError::OutOfRegion { addr, len })
        }
    }

    fn read_bytes(&self, addr: GuestAddress, buf: &mut [u8]) -> Result<(), GuestMemoryError> {
        let len =
            u64::try_from(buf.len()).map_err(|_| GuestMemoryError::Overflow { addr, len: 0 })?;
        self.check_range(addr, len)?;
        let offset = self
            .layout
            .host_offset(addr.raw())
            .ok_or(GuestMemoryError::OutOfRegion { addr, len })?;
        if self.mapping.read(offset, buf) {
            Ok(())
        } else {
            Err(GuestMemoryError::OutOfRegion { addr, len })
        }
    }

    fn write_bytes(&self, addr: GuestAddress, bytes: &[u8]) -> Result<(), GuestMemoryError> {
        let len =
            u64::try_from(bytes.len()).map_err(|_| GuestMemoryError::Overflow { addr, len: 0 })?;
        self.check_range(addr, len)?;
        let offset = self
            .layout
            .host_offset(addr.raw())
            .ok_or(GuestMemoryError::OutOfRegion { addr, len })?;
        if self.mapping.write(offset, bytes) {
            Ok(())
        } else {
            Err(GuestMemoryError::OutOfRegion { addr, len })
        }
    }
}
