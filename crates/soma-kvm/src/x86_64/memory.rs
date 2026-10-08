//! Page-aligned private anonymous guest RAM owned by one machine and shared with its devices.
//!
//! [`GuestRam`] is the loader's exclusive view while no vCPU runs. [`SharedRam`] is the
//! range-checked [`GuestMemory`] view the device thread and the vCPU thread use afterwards;
//! it keeps the mapping alive and never forms a Rust reference over guest bytes.

use std::sync::Arc;

use kvm_ioctls::VmFd;

use super::{
    error::{MachineError, Phase},
    layout::GuestLayout,
};

mod mapping;
mod shared;

pub(crate) use mapping::RamMapping;

pub use shared::SharedRam;

/// One private, lazily populated guest RAM mapping registered as KVM memory slot 0 at GPA 0.
pub(crate) struct GuestRam {
    mapping: Arc<RamMapping>,
    layout: GuestLayout,
}

impl GuestRam {
    pub(crate) fn map(layout: GuestLayout) -> Result<Self, MachineError> {
        let length = usize::try_from(layout.ram_bytes())
            .map_err(|_| MachineError::invalid(Phase::MapMemory, "guest RAM exceeds usize"))?;
        Ok(Self {
            mapping: Arc::new(RamMapping::anonymous(length, Phase::MapMemory)?),
            layout,
        })
    }

    /// Wraps a mapping the caller already produced for exactly `layout.ram_bytes()` bytes.
    pub(super) fn from_mapping(
        mapping: RamMapping,
        layout: GuestLayout,
    ) -> Result<Self, MachineError> {
        if u64::try_from(mapping.len()).is_ok_and(|len| len == layout.ram_bytes()) {
            Ok(Self {
                mapping: Arc::new(mapping),
                layout,
            })
        } else {
            Err(MachineError::invalid(
                Phase::MapMemory,
                "restored mapping length does not match the certified guest RAM size",
            ))
        }
    }

    pub(crate) const fn layout(&self) -> GuestLayout {
        self.layout
    }

    /// Copies `bytes` to guest-physical `address`, rejecting any byte outside RAM.
    pub(crate) fn write(&mut self, address: u64, bytes: &[u8]) -> Result<(), MachineError> {
        let length = u64::try_from(bytes.len())
            .map_err(|_| MachineError::invalid(Phase::LoadGuest, "guest write length overflow"))?;
        let Some(offset) = self.object_offset(address, length)? else {
            return Ok(());
        };
        if self.mapping.write(offset, bytes) {
            Ok(())
        } else {
            Err(MachineError::invalid(
                Phase::LoadGuest,
                "guest write is outside registered RAM",
            ))
        }
    }

    /// Zero-fills `[address, address + length)`, rejecting any byte outside RAM.
    pub(crate) fn zero(&mut self, address: u64, length: u64) -> Result<(), MachineError> {
        let Some(offset) = self.object_offset(address, length)? else {
            return Ok(());
        };
        let count = usize::try_from(length)
            .map_err(|_| MachineError::invalid(Phase::LoadGuest, "zero-fill length overflow"))?;
        if self.mapping.zero(offset, count) {
            Ok(())
        } else {
            Err(MachineError::invalid(
                Phase::LoadGuest,
                "guest zero-fill is outside registered RAM",
            ))
        }
    }

    /// Translates a guest range to an offset in the memory object.
    ///
    /// A zero-length range is a no-op and returns `None`; any other range must lie inside one
    /// backed range, so a write that straddles the MMIO hole is refused rather than silently
    /// landing in the wrong place.
    fn object_offset(&self, address: u64, length: u64) -> Result<Option<u64>, MachineError> {
        if length == 0 {
            return Ok(None);
        }
        self.layout
            .host_offset(address)
            .filter(|_| self.layout.contains(address, length))
            .map(Some)
            .ok_or_else(|| {
                MachineError::invalid(Phase::LoadGuest, "guest range is outside registered RAM")
            })
    }

    /// Registers every backed RAM range as its own KVM memory slot.
    ///
    /// A machine whose RAM ends at or below the MMIO boundary has one range from address zero,
    /// which is the version 1 single-slot layout. A larger machine has that range plus one above
    /// the hole, and each range is registered at the guest address and object offset the layout
    /// derived, not at an address a caller chose.
    pub(crate) fn register(&self, vm: &VmFd) -> Result<(), MachineError> {
        for region in self.layout.regions() {
            self.mapping.register_range(
                vm,
                region.slot,
                region.guest_start,
                region.host_offset,
                region.size,
                Phase::RegisterMemory,
            )?;
        }
        Ok(())
    }

    /// A range-checked device view that keeps the mapping alive.
    pub(crate) fn shared(&self) -> SharedRam {
        SharedRam {
            mapping: Arc::clone(&self.mapping),
            layout: self.layout,
        }
    }
}

#[cfg(test)]
mod tests;
