//! Page-aligned private anonymous guest RAM owned by one machine.

use std::ptr;

use kvm_bindings::kvm_userspace_memory_region;
use kvm_ioctls::VmFd;

use super::{
    error::{MachineError, Phase},
    layout::GuestLayout,
};

/// One private, lazily populated guest RAM mapping registered as KVM memory slot 0 at GPA 0.
pub(crate) struct GuestRam {
    base: ptr::NonNull<u8>,
    layout: GuestLayout,
}

impl GuestRam {
    #[allow(unsafe_code)]
    pub(crate) fn map(layout: GuestLayout) -> Result<Self, MachineError> {
        let length = usize::try_from(layout.ram_bytes())
            .map_err(|_| MachineError::invalid(Phase::MapMemory, "guest RAM exceeds usize"))?;
        // SAFETY: An anonymous private mapping with a null hint has no aliasing requirements.
        // The returned pointer is checked against MAP_FAILED before it is retained, and the
        // mapping is unmapped exactly once in `Drop` after the VM that references it is gone.
        let raw = unsafe {
            libc::mmap(
                ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
                -1,
                0,
            )
        };
        if raw == libc::MAP_FAILED {
            return Err(MachineError::last_os(Phase::MapMemory));
        }
        let base = ptr::NonNull::new(raw.cast::<u8>())
            .ok_or_else(|| MachineError::invalid(Phase::MapMemory, "mmap returned null"))?;
        Ok(Self { base, layout })
    }

    pub(crate) const fn layout(&self) -> GuestLayout {
        self.layout
    }

    /// Copies `bytes` to guest-physical `address`, rejecting any byte outside RAM.
    #[allow(unsafe_code)]
    pub(crate) fn write(&mut self, address: u64, bytes: &[u8]) -> Result<(), MachineError> {
        let length = u64::try_from(bytes.len())
            .map_err(|_| MachineError::invalid(Phase::LoadGuest, "write length overflow"))?;
        if !self.layout.contains(address, length) {
            return Err(MachineError::invalid(
                Phase::LoadGuest,
                "guest write is outside registered RAM",
            ));
        }
        let offset = usize::try_from(address)
            .map_err(|_| MachineError::invalid(Phase::LoadGuest, "guest address overflow"))?;
        // SAFETY: `contains` proved `[address, address + len)` lies inside the live mapping of
        // `layout.ram_bytes()` bytes starting at `base`, so `base + offset` is in bounds for
        // `bytes.len()` bytes. The source is a distinct Rust slice, so the ranges do not overlap.
        // No vCPU runs before loading completes, so there is no concurrent guest access.
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), self.base.as_ptr().add(offset), bytes.len());
        }
        Ok(())
    }

    /// Zero-fills `[address, address + length)`, rejecting any byte outside RAM.
    #[allow(unsafe_code)]
    pub(crate) fn zero(&mut self, address: u64, length: u64) -> Result<(), MachineError> {
        if !self.layout.contains(address, length) {
            return Err(MachineError::invalid(
                Phase::LoadGuest,
                "guest zero-fill is outside registered RAM",
            ));
        }
        let offset = usize::try_from(address)
            .map_err(|_| MachineError::invalid(Phase::LoadGuest, "guest address overflow"))?;
        let count = usize::try_from(length)
            .map_err(|_| MachineError::invalid(Phase::LoadGuest, "zero-fill length overflow"))?;
        // SAFETY: `contains` proved `[address, address + length)` lies inside the live mapping,
        // so `base + offset` is valid for `count` bytes, and no vCPU runs before loading ends.
        unsafe {
            ptr::write_bytes(self.base.as_ptr().add(offset), 0, count);
        }
        Ok(())
    }

    /// Registers the whole mapping as KVM user-memory slot 0 at guest-physical address 0.
    #[allow(unsafe_code)]
    pub(crate) fn register(&self, vm: &VmFd) -> Result<(), MachineError> {
        let userspace_addr = u64::try_from(self.base.as_ptr().addr())
            .map_err(|_| MachineError::invalid(Phase::RegisterMemory, "host address overflow"))?;
        let region = kvm_userspace_memory_region {
            slot: 0,
            flags: 0,
            guest_phys_addr: 0,
            memory_size: self.layout.ram_bytes(),
            userspace_addr,
        };
        // SAFETY: Slot 0 is registered exactly once and covers precisely this mapping. The
        // orchestrator drops the vCPU and the VM before this `GuestRam`, so KVM never references
        // the range after it is unmapped.
        unsafe { vm.set_user_memory_region(region) }
            .map_err(|error| MachineError::os(Phase::RegisterMemory, error))
    }
}

impl Drop for GuestRam {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        let Ok(length) = usize::try_from(self.layout.ram_bytes()) else {
            return;
        };
        // SAFETY: `base` and `length` are exactly the values returned by and passed to the
        // successful `mmap` in `map`, and this is the only unmap of that mapping.
        let _ignored = unsafe { libc::munmap(self.base.as_ptr().cast(), length) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::x86_64::layout::{KERNEL_START, MIN_RAM_BYTES};

    #[test]
    fn maps_writes_and_rejects_out_of_range_writes() {
        let mut ram = GuestRam::map(GuestLayout::new(MIN_RAM_BYTES).unwrap()).unwrap();
        ram.write(KERNEL_START, b"SOMA").unwrap();
        ram.write(MIN_RAM_BYTES - 4, b"SOMA").unwrap();
        assert!(ram.write(MIN_RAM_BYTES - 3, b"SOMA").is_err());
        assert!(ram.write(u64::MAX, b"S").is_err());
        assert_eq!(ram.layout().ram_bytes(), MIN_RAM_BYTES);
    }
}
