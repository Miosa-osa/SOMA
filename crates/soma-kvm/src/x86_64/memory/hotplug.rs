//! The hotplug region: an anonymous, lazily populated mapping of its own behind one extra KVM
//! memory slot, which the guest grows and shrinks through the memory device.
//!
//! The region is never part of boot RAM or the snapshot memory object. Which blocks are plugged
//! is the memory device's state, not this mapping's: the guest can address the whole slot, and
//! the device is what keeps it cooperative. What the host does own is the other direction. An
//! unplugged block is handed back through [`HotplugBacking`], which drops its pages at once, so
//! the host gets the memory back and a later plug reads zero.

use std::{io, sync::Arc};

use kvm_ioctls::VmFd;

use super::{
    super::error::{MachineError, Phase},
    RamMapping,
};
use crate::memory_layout::HotplugRegion;
use crate::virtio::RegionBacking;

/// The mapping behind the hotplug slot, shared by the device view and the backing.
pub struct HotplugRam {
    mapping: RamMapping,
    region: HotplugRegion,
}

impl HotplugRam {
    /// Maps the region empty: no host page is resident until the guest touches one.
    pub(crate) fn map(region: HotplugRegion) -> Result<Self, MachineError> {
        let length = usize::try_from(region.size)
            .map_err(|_| MachineError::invalid(Phase::MapMemory, "hotplug size exceeds usize"))?;
        Ok(Self {
            mapping: RamMapping::anonymous(length, Phase::MapMemory)?,
            region,
        })
    }

    /// Registers the whole region as its own KVM memory slot.
    pub(crate) fn register(&self, vm: &VmFd) -> Result<(), MachineError> {
        self.mapping
            .register_range(vm, &self.region.ram_region(), Phase::RegisterMemory)
    }

    /// The region's geometry, which is all a snapshot records about it.
    #[must_use]
    pub const fn region(&self) -> HotplugRegion {
        self.region
    }

    /// Reads bytes at a guest-physical address inside the region.
    pub(crate) fn read(&self, address: u64, buf: &mut [u8]) -> bool {
        let length = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        self.region
            .contains(address, length)
            .then(|| self.region.offset_of(address))
            .flatten()
            .is_some_and(|offset| self.mapping.read(offset, buf))
    }

    /// Writes bytes at a guest-physical address inside the region.
    pub(crate) fn write(&self, address: u64, bytes: &[u8]) -> bool {
        let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        self.region
            .contains(address, length)
            .then(|| self.region.offset_of(address))
            .flatten()
            .is_some_and(|offset| self.mapping.write(offset, bytes))
    }

    /// Host memory the region currently holds, in bytes.
    ///
    /// # Errors
    ///
    /// Returns the host failure of the residency query.
    pub fn resident_bytes(&self) -> io::Result<u64> {
        self.mapping.resident_bytes().map_err(io::Error::other)
    }
}

/// Hands unplugged blocks back to the host.
pub struct HotplugBacking {
    ram: Arc<HotplugRam>,
}

impl HotplugBacking {
    pub(crate) const fn new(ram: Arc<HotplugRam>) -> Self {
        Self { ram }
    }

    /// Host memory the region currently holds, in bytes, for the host's accounting.
    ///
    /// # Errors
    ///
    /// Returns the host failure of the residency query.
    pub fn resident_bytes(&self) -> io::Result<u64> {
        self.ram.resident_bytes()
    }
}

impl RegionBacking for HotplugBacking {
    fn discard(&mut self, offset: u64, len: u64) -> io::Result<()> {
        let count = usize::try_from(len)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "discard length"))?;
        self.ram
            .mapping
            .discard(offset, count)
            .map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests;
