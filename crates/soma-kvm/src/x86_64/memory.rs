//! Page-aligned private anonymous guest RAM owned by one machine and shared with its devices.
//!
//! [`GuestRam`] is the loader's exclusive view while no vCPU runs. [`SharedRam`] is the
//! range-checked [`GuestMemory`] view the device thread and the vCPU thread use afterwards;
//! it keeps the mapping alive and never forms a Rust reference over guest bytes.

use std::{ptr, sync::Arc};

use kvm_bindings::kvm_userspace_memory_region;
use kvm_ioctls::VmFd;

use super::{
    error::{MachineError, Phase},
    layout::GuestLayout,
};
use crate::virtio::{GuestAddress, GuestMemory, GuestMemoryError};

/// One anonymous private mapping unmapped exactly once when the last owner drops it.
pub(crate) struct RamMapping {
    base: ptr::NonNull<u8>,
    len: usize,
}

// SAFETY: The mapping is a plain byte region touched only through raw-pointer copies whose
// bounds are checked against `len`; no thread holds a Rust reference into it, so moving or
// sharing the handle between threads creates no aliasing that the type system must forbid.
#[allow(unsafe_code)]
unsafe impl Send for RamMapping {}
// SAFETY: See the `Send` justification; concurrent access is bounded raw-pointer I/O only.
#[allow(unsafe_code)]
unsafe impl Sync for RamMapping {}

impl RamMapping {
    #[allow(unsafe_code)]
    pub(crate) fn anonymous(len: usize, phase: Phase) -> Result<Self, MachineError> {
        // SAFETY: An anonymous private mapping with a null hint has no aliasing requirements.
        // The returned pointer is checked against MAP_FAILED before it is retained, and the
        // mapping is unmapped exactly once in `Drop` after every KVM slot referencing it is
        // gone, because the machine drops its VM before its last `RamMapping` owner.
        let raw = unsafe {
            libc::mmap(
                ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
                -1,
                0,
            )
        };
        if raw == libc::MAP_FAILED {
            return Err(MachineError::last_os(phase));
        }
        let base = ptr::NonNull::new(raw.cast::<u8>())
            .ok_or_else(|| MachineError::invalid(phase, "mmap returned null"))?;
        Ok(Self { base, len })
    }

    /// Takes ownership of a range another mapper produced, unmapping it exactly once on drop.
    ///
    /// Snapshot restore maps `memory.raw` with `MAP_PRIVATE | MAP_NORESERVE` through the
    /// snapshot codec and hands the range here, so the machine keeps one owner for the KVM
    /// slot, the device view, and the final `munmap`.
    pub(super) const fn adopt(base: ptr::NonNull<u8>, len: usize) -> Self {
        Self { base, len }
    }

    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn host_address(&self) -> Result<u64, MachineError> {
        u64::try_from(self.base.as_ptr().addr())
            .map_err(|_| MachineError::invalid(Phase::RegisterMemory, "host address overflow"))
    }

    fn range(&self, offset: u64, len: usize) -> Option<usize> {
        let offset = usize::try_from(offset).ok()?;
        let end = offset.checked_add(len)?;
        (end <= self.len).then_some(offset)
    }

    /// Copies guest bytes at `offset` into `buf` after a bounds check.
    #[allow(unsafe_code)]
    pub(crate) fn read(&self, offset: u64, buf: &mut [u8]) -> bool {
        let Some(start) = self.range(offset, buf.len()) else {
            return false;
        };
        // SAFETY: `range` proved `[start, start + buf.len())` lies inside the live mapping, and
        // `buf` is a distinct host slice, so the regions cannot overlap. The guest may write
        // the same bytes concurrently; a torn copy is hostile input for the checked parsers,
        // never a memory-safety violation, because no reference into the mapping exists.
        unsafe {
            ptr::copy_nonoverlapping(self.base.as_ptr().add(start), buf.as_mut_ptr(), buf.len());
        }
        true
    }

    /// Copies `bytes` to guest offset `offset` after a bounds check.
    #[allow(unsafe_code)]
    pub(crate) fn write(&self, offset: u64, bytes: &[u8]) -> bool {
        let Some(start) = self.range(offset, bytes.len()) else {
            return false;
        };
        // SAFETY: `range` proved the destination lies inside the live mapping and `bytes` is a
        // distinct host slice; the guest observes the copy as ordinary shared memory.
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), self.base.as_ptr().add(start), bytes.len());
        }
        true
    }

    /// Reads `buf.len()` bytes at `offset` with volatile loads so a concurrently written page
    /// is observed as it is now rather than as a cached earlier value.
    #[allow(unsafe_code)]
    pub(crate) fn read_volatile(&self, offset: u64, buf: &mut [u8]) -> bool {
        let Some(start) = self.range(offset, buf.len()) else {
            return false;
        };
        for (index, byte) in buf.iter_mut().enumerate() {
            // SAFETY: `range` proved `start + buf.len()` stays inside the live mapping, so every
            // `start + index` is a valid one-byte volatile read.
            *byte = unsafe { ptr::read_volatile(self.base.as_ptr().add(start + index)) };
        }
        true
    }

    /// Zero-fills `count` bytes at `offset` after a bounds check.
    #[allow(unsafe_code)]
    pub(crate) fn zero(&self, offset: u64, count: usize) -> bool {
        let Some(start) = self.range(offset, count) else {
            return false;
        };
        // SAFETY: `range` proved `[start, start + count)` lies inside the live mapping.
        unsafe {
            ptr::write_bytes(self.base.as_ptr().add(start), 0, count);
        }
        true
    }

    /// Registers the whole mapping as KVM user-memory `slot` at `guest_phys_addr`.
    #[allow(unsafe_code)]
    pub(crate) fn register(
        &self,
        vm: &VmFd,
        slot: u32,
        guest_phys_addr: u64,
        phase: Phase,
    ) -> Result<(), MachineError> {
        let region = kvm_userspace_memory_region {
            slot,
            flags: 0,
            guest_phys_addr,
            memory_size: u64::try_from(self.len)
                .map_err(|_| MachineError::invalid(phase, "mapping length overflow"))?,
            userspace_addr: self.host_address()?,
        };
        // SAFETY: The slot covers precisely this live mapping. The machine drops its vCPU and
        // VM, or retires the slot with a zero-length region, before the mapping is unmapped,
        // so KVM never references the range after `munmap`.
        unsafe { vm.set_user_memory_region(region) }.map_err(|error| MachineError::os(phase, error))
    }
}

impl Drop for RamMapping {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: `base` and `len` are exactly the values returned by and passed to the
        // successful `mmap` in `anonymous`, and this is the only unmap of that mapping.
        let _ignored = unsafe { libc::munmap(self.base.as_ptr().cast(), self.len) };
    }
}

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
        if self.mapping.write(address, bytes) {
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
        let count = usize::try_from(length)
            .map_err(|_| MachineError::invalid(Phase::LoadGuest, "zero-fill length overflow"))?;
        if self.mapping.zero(address, count) {
            Ok(())
        } else {
            Err(MachineError::invalid(
                Phase::LoadGuest,
                "guest zero-fill is outside registered RAM",
            ))
        }
    }

    /// Registers the whole mapping as KVM user-memory slot 0 at guest-physical address 0.
    pub(crate) fn register(&self, vm: &VmFd) -> Result<(), MachineError> {
        self.mapping.register(vm, 0, 0, Phase::RegisterMemory)
    }

    /// A range-checked device view that keeps the mapping alive.
    pub(crate) fn shared(&self) -> SharedRam {
        SharedRam(Arc::clone(&self.mapping))
    }
}

/// The checked guest-physical view used by the virtio devices and the MMIO dispatcher.
#[derive(Clone)]
pub struct SharedRam(Arc<RamMapping>);

impl GuestMemory for SharedRam {
    fn check_range(&self, addr: GuestAddress, len: u64) -> Result<(), GuestMemoryError> {
        let end = addr
            .checked_add(len)
            .ok_or(GuestMemoryError::Overflow { addr, len })?;
        if u64::try_from(self.0.len()).is_ok_and(|ram| end.raw() <= ram) {
            Ok(())
        } else {
            Err(GuestMemoryError::OutOfRegion { addr, len })
        }
    }

    fn read_bytes(&self, addr: GuestAddress, buf: &mut [u8]) -> Result<(), GuestMemoryError> {
        let len =
            u64::try_from(buf.len()).map_err(|_| GuestMemoryError::Overflow { addr, len: 0 })?;
        self.check_range(addr, len)?;
        if self.0.read(addr.raw(), buf) {
            Ok(())
        } else {
            Err(GuestMemoryError::OutOfRegion { addr, len })
        }
    }

    fn write_bytes(&self, addr: GuestAddress, bytes: &[u8]) -> Result<(), GuestMemoryError> {
        let len =
            u64::try_from(bytes.len()).map_err(|_| GuestMemoryError::Overflow { addr, len: 0 })?;
        self.check_range(addr, len)?;
        if self.0.write(addr.raw(), bytes) {
            Ok(())
        } else {
            Err(GuestMemoryError::OutOfRegion { addr, len })
        }
    }
}

#[cfg(test)]
mod tests;
