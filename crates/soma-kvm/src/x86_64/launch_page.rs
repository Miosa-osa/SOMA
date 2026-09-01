//! The dedicated 4 KiB launch-page memory slot: mapped and registered before the vCPU runs,
//! written once with the host launch material, verified erased after the guest consumed it,
//! and retired with a zero-length `KVM_SET_USER_MEMORY_REGION` so no snapshot can ever
//! contain it.
//!
//! Retirement is two acts, and only the first of them is what makes repair irreversible: the
//! host copy is erased, and then the slot is removed. Removing a memory slot from a running
//! VM invalidates that VM's whole extended page table, so the guest re-faults every page it
//! still has live, which is measurable in the resume. The erasure may therefore be separated
//! from the removal, and a diagnostic exists to measure what separating them is worth.

use kvm_bindings::kvm_userspace_memory_region;
use kvm_ioctls::VmFd;

use super::{
    error::{MachineError, MachineErrorKind, Phase},
    memory::RamMapping,
};

/// Guest-physical address of the launch page; above RAM and the five MMIO pages.
///
/// This restates `soma_guest::LAUNCH_PAGE_GUEST_ADDRESS`; the live test binds the two values.
pub const LAUNCH_PAGE_GPA: u64 = 0xd010_0000;
/// KVM memory slot of the launch page; slot 0 is guest RAM.
pub const LAUNCH_PAGE_SLOT: u32 = 1;
/// Exact size of the launch page.
pub const LAUNCH_PAGE_SIZE: usize = 4096;

/// Names the diagnostic that separates the erasure from the removal, so the cost of removing a
/// memory slot from a running guest can be measured. Never set on a host serving requests.
const DEFER_REMOVAL: &str = "SOMA_KVM_DEFER_LAUNCH_PAGE_SLOT";

/// What one call to [`LaunchPageSlot::retire_step`] did.
pub(crate) struct RetireStep {
    /// Whether this call erased the material, which is what commits the repair.
    pub(crate) committed: bool,
    /// Whether the slot is now removed from the VM and the page may be dropped.
    pub(crate) removed: bool,
    /// `LaunchPageNotErased` when material remained, or the removal's own failure.
    pub(crate) outcome: Result<(), MachineError>,
}

/// How far retirement has got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Erasure {
    /// The material is still in the mapping.
    Pending,
    /// The host copy is zeroed; the flag is whether the guest had zeroed it first.
    Done(bool),
}

/// One mapped and registered launch page.
pub(crate) struct LaunchPageSlot {
    mapping: RamMapping,
    registered: bool,
    erasure: Erasure,
    defer_removal: bool,
}

impl LaunchPageSlot {
    /// Maps a fresh zero page and registers it as [`LAUNCH_PAGE_SLOT`].
    pub(crate) fn map_and_register(vm: &VmFd) -> Result<Self, MachineError> {
        let mapping = RamMapping::anonymous(LAUNCH_PAGE_SIZE, Phase::LaunchPage)?;
        mapping.register(vm, LAUNCH_PAGE_SLOT, LAUNCH_PAGE_GPA, Phase::LaunchPage)?;
        Ok(Self {
            mapping,
            registered: true,
            erasure: Erasure::Pending,
            defer_removal: std::env::var_os(DEFER_REMOVAL).is_some(),
        })
    }

    /// Publishes the launch material; the caller erases its own copy afterwards.
    pub(crate) fn write(&mut self, page: &[u8; LAUNCH_PAGE_SIZE]) -> Result<(), MachineError> {
        if self.mapping.write(0, page) {
            Ok(())
        } else {
            Err(MachineError::invalid(
                Phase::LaunchPage,
                "launch page write is outside the slot",
            ))
        }
    }

    /// Whether every byte of the page currently reads as zero.
    pub(crate) fn is_erased(&self) -> bool {
        let mut page = [0_u8; LAUNCH_PAGE_SIZE];
        self.mapping.read_volatile(0, &mut page) && page.iter().all(|byte| *byte == 0)
    }

    /// Whether the first `prefix.len()` bytes still equal `prefix`.
    pub(crate) fn starts_with(&self, prefix: &[u8]) -> bool {
        let mut head = vec![0_u8; prefix.len()];
        self.mapping.read_volatile(0, &mut head) && head == prefix
    }

    /// Erases the material, then removes the slot, and reports whether the slot is now gone.
    ///
    /// The host copy is zeroed on the first call even when the guest failed to erase the page,
    /// so the material never outlives that call; the verification result is returned
    /// afterwards. The removal follows in the same call unless an operator deferred it, in
    /// which case a later call performs it and the caller holds the slot until then.
    pub(crate) fn retire_step(&mut self, vm: &VmFd) -> RetireStep {
        let committed = self.erasure == Erasure::Pending;
        if self.erase_once() {
            return RetireStep {
                committed,
                removed: false,
                outcome: self.verdict(),
            };
        }
        let removal = self.unregister(vm);
        RetireStep {
            committed,
            removed: true,
            outcome: removal.and(self.verdict()),
        }
    }

    /// Erases the host copy on the first call, and reports whether removal is being deferred.
    fn erase_once(&mut self) -> bool {
        if self.erasure != Erasure::Pending {
            return false;
        }
        self.erasure = Erasure::Done(self.is_erased());
        self.mapping.zero(0, LAUNCH_PAGE_SIZE);
        self.defer_removal
    }

    /// Whether the guest had erased the page before the host did.
    fn verdict(&self) -> Result<(), MachineError> {
        if self.erasure == Erasure::Done(true) {
            Ok(())
        } else {
            Err(MachineError::new(
                Phase::LaunchPage,
                MachineErrorKind::LaunchPageNotErased,
            ))
        }
    }

    #[allow(unsafe_code)]
    fn unregister(&mut self, vm: &VmFd) -> Result<(), MachineError> {
        if !self.registered {
            return Ok(());
        }
        let region = kvm_userspace_memory_region {
            slot: LAUNCH_PAGE_SLOT,
            flags: 0,
            guest_phys_addr: LAUNCH_PAGE_GPA,
            memory_size: 0,
            userspace_addr: 0,
        };
        // SAFETY: A zero-length region deletes the slot; KVM stops referencing the mapping
        // before the ioctl returns, and the mapping is unmapped only after this call.
        unsafe { vm.set_user_memory_region(region) }
            .map_err(|error| MachineError::os(Phase::LaunchPage, error))?;
        self.registered = false;
        Ok(())
    }
}

impl Drop for LaunchPageSlot {
    fn drop(&mut self) {
        // A slot dropped without `retire` (a failed create) is zeroed so the mapping never
        // releases material; the VM it was registered with is being dropped in the same unwind.
        self.mapping.zero(0, LAUNCH_PAGE_SIZE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_constants_sit_above_the_mmio_window() {
        assert_eq!(LAUNCH_PAGE_GPA, 0xd010_0000);
        assert!(LAUNCH_PAGE_GPA > crate::virtio::Slot::Rng.end());
        assert_eq!(LAUNCH_PAGE_GPA % 4096, 0);
        assert_eq!(LAUNCH_PAGE_SLOT, 1);
        assert_eq!(LAUNCH_PAGE_SIZE, 4096);
    }

    #[test]
    fn a_written_page_is_not_erased_until_zeroed() {
        let mut slot = LaunchPageSlot {
            mapping: RamMapping::anonymous(LAUNCH_PAGE_SIZE, Phase::LaunchPage).unwrap(),
            registered: false,
            erasure: Erasure::Pending,
            defer_removal: false,
        };
        assert!(slot.is_erased());
        let mut page = [0_u8; LAUNCH_PAGE_SIZE];
        page[..4].copy_from_slice(b"SOMA");
        page[LAUNCH_PAGE_SIZE - 1] = 7;
        slot.write(&page).unwrap();
        assert!(!slot.is_erased());
        assert!(slot.starts_with(b"SOMA"));
        assert!(!slot.starts_with(b"XOMA"));
        slot.mapping.zero(0, LAUNCH_PAGE_SIZE);
        assert!(slot.is_erased());
    }

    #[test]
    fn a_deferred_removal_erases_first_and_removes_on_the_next_call() {
        let mut slot = LaunchPageSlot {
            mapping: RamMapping::anonymous(LAUNCH_PAGE_SIZE, Phase::LaunchPage).unwrap(),
            registered: false,
            erasure: Erasure::Pending,
            defer_removal: true,
        };
        let mut page = [0_u8; LAUNCH_PAGE_SIZE];
        page[..4].copy_from_slice(b"SOMA");
        slot.write(&page).unwrap();

        // The first call erases the material and holds the slot back.
        assert!(slot.erase_once());
        assert!(slot.is_erased());
        assert_eq!(slot.erasure, Erasure::Done(false));
        assert!(slot.verdict().is_err(), "the guest had not erased the page");

        // The second call erases nothing more and lets the slot go.
        assert!(!slot.erase_once());
    }

    #[test]
    fn an_undeferred_removal_erases_and_releases_in_one_call() {
        let mut slot = LaunchPageSlot {
            mapping: RamMapping::anonymous(LAUNCH_PAGE_SIZE, Phase::LaunchPage).unwrap(),
            registered: false,
            erasure: Erasure::Pending,
            defer_removal: false,
        };
        assert!(!slot.erase_once());
        assert_eq!(slot.erasure, Erasure::Done(true));
        assert!(slot.verdict().is_ok(), "the page was already zero");
    }
}
