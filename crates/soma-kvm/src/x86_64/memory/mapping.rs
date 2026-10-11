//! One anonymous or file-backed private mapping, unmapped exactly once when it is dropped.
//!
//! The mapping is a plain byte region touched only through raw-pointer copies whose bounds are
//! checked against its length, and no thread ever holds a Rust reference into it, so it is
//! shared between the vCPU thread, the device thread, and the capture reader without aliasing
//! the type system must forbid.

use std::ptr;

use kvm_bindings::kvm_userspace_memory_region;
use kvm_ioctls::VmFd;

use super::super::error::{MachineError, Phase};
use crate::memory_layout::RamRegion;

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

    /// Opts the mapping out of transparent huge pages.
    ///
    /// A region whose blocks the guest plugs and unplugs two mebibytes at a time must return a
    /// block's memory when it is discarded, and a huge page would keep a whole 2 MiB resident
    /// for one touched byte. Best effort: a kernel without huge pages refuses the advice and
    /// has nothing to opt out of.
    #[allow(unsafe_code)]
    pub(crate) fn no_huge_pages(&self) {
        // SAFETY: `base` and `len` are exactly the live mapping; the advice changes only how the
        // kernel backs it and never unmaps or moves it.
        let _ignored =
            unsafe { libc::madvise(self.base.as_ptr().cast(), self.len, libc::MADV_NOHUGEPAGE) };
    }

    /// Takes ownership of a range another mapper produced, unmapping it exactly once on drop.
    ///
    /// Snapshot restore maps `memory.raw` with `MAP_PRIVATE | MAP_NORESERVE` through the
    /// snapshot codec and hands the range here, so the machine keeps one owner for the KVM
    /// slot, the device view, and the final `munmap`.
    pub(crate) const fn adopt(base: ptr::NonNull<u8>, len: usize) -> Self {
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

    /// Returns `count` bytes at `offset` to the host and makes them read as zero again.
    ///
    /// `madvise(MADV_DONTNEED)` on a private anonymous mapping drops the resident pages at once,
    /// and the next touch faults in a fresh zero page, so a discarded range costs no memory and
    /// leaks no earlier content. `offset` and `count` must be whole pages inside the mapping.
    #[allow(unsafe_code)]
    pub(crate) fn discard(&self, offset: u64, count: usize) -> Result<(), MachineError> {
        let page = crate::memory_layout::PAGE_SIZE;
        let aligned = offset.is_multiple_of(page)
            && u64::try_from(count).is_ok_and(|c| c.is_multiple_of(page));
        let Some(start) = self.range(offset, count).filter(|_| aligned && count > 0) else {
            return Err(MachineError::invalid(
                Phase::MapMemory,
                "discard range is not whole pages inside the mapping",
            ));
        };
        // SAFETY: `range` proved `[start, start + count)` lies inside the live mapping and the
        // alignment check proved it is whole pages. `MADV_DONTNEED` on a private anonymous
        // mapping only drops pages; it never unmaps, so every pointer into it stays valid.
        let rc = unsafe {
            libc::madvise(
                self.base.as_ptr().add(start).cast(),
                count,
                libc::MADV_DONTNEED,
            )
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(MachineError::last_os(Phase::MapMemory))
        }
    }

    /// Bytes of the mapping currently resident in host memory, counted a gigabyte at a time.
    #[allow(unsafe_code)]
    pub(crate) fn resident_bytes(&self) -> Result<u64, MachineError> {
        const WINDOW: usize = 1 << 30;
        let page = usize::try_from(crate::memory_layout::PAGE_SIZE).unwrap_or(4096);
        let mut resident = 0_u64;
        let mut start = 0_usize;
        let mut vec = vec![0_u8; WINDOW.min(self.len).div_ceil(page)];
        while start < self.len {
            let span = WINDOW.min(self.len - start);
            // SAFETY: `[start, start + span)` lies inside the live mapping, `start` is a
            // multiple of the window and so of the page size, and `vec` holds one byte per
            // page of the span.
            let rc = unsafe {
                libc::mincore(
                    self.base.as_ptr().add(start).cast(),
                    span,
                    vec.as_mut_ptr().cast(),
                )
            };
            if rc != 0 {
                return Err(MachineError::last_os(Phase::MapMemory));
            }
            let pages = span.div_ceil(page);
            resident += vec[..pages].iter().filter(|byte| **byte & 1 == 1).count() as u64;
            start += span;
        }
        Ok(resident * crate::memory_layout::PAGE_SIZE)
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
        let size = u64::try_from(self.len)
            .map_err(|_| MachineError::invalid(phase, "mapping length overflow"))?;
        self.register_range(
            vm,
            &RamRegion {
                slot,
                guest_start: guest_phys_addr,
                host_offset: 0,
                size,
            },
            phase,
        )
    }

    /// Registers one backed range of guest RAM as one KVM user-memory slot.
    ///
    /// A machine whose RAM ends above the MMIO boundary has two ranges in one object, so a slot
    /// covers a slice of the mapping rather than the whole of it; the slice is proved inside the
    /// mapping before any byte of it is published to KVM.
    #[allow(unsafe_code)]
    pub(crate) fn register_range(
        &self,
        vm: &VmFd,
        region: &RamRegion,
        phase: Phase,
    ) -> Result<(), MachineError> {
        let RamRegion {
            slot,
            guest_start: guest_phys_addr,
            host_offset,
            size,
        } = *region;
        let length = usize::try_from(size)
            .map_err(|_| MachineError::invalid(phase, "mapping slice length overflow"))?;
        if self.range(host_offset, length).is_none() {
            return Err(MachineError::invalid(
                phase,
                "mapping slice leaves the registered mapping",
            ));
        }
        let base = self
            .host_address()?
            .checked_add(host_offset)
            .ok_or_else(|| MachineError::invalid(phase, "host address overflow"))?;
        let region = kvm_userspace_memory_region {
            slot,
            flags: 0,
            guest_phys_addr,
            memory_size: size,
            userspace_addr: base,
        };
        // SAFETY: The slice was proved inside this live mapping. The machine drops its vCPU and
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
