//! The dedicated 4 KiB launch-page memory slot: mapped and registered before the vCPU runs,
//! written once with the host launch material, verified erased after the guest consumed it,
//! and retired with a zero-length `KVM_SET_USER_MEMORY_REGION` so no snapshot can ever
//! contain it.

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

/// One mapped and registered launch page.
pub(crate) struct LaunchPageSlot {
    mapping: RamMapping,
    registered: bool,
}

impl LaunchPageSlot {
    /// Maps a fresh zero page and registers it as [`LAUNCH_PAGE_SLOT`].
    pub(crate) fn map_and_register(vm: &VmFd) -> Result<Self, MachineError> {
        let mapping = RamMapping::anonymous(LAUNCH_PAGE_SIZE, Phase::LaunchPage)?;
        mapping.register(vm, LAUNCH_PAGE_SLOT, LAUNCH_PAGE_GPA, Phase::LaunchPage)?;
        Ok(Self {
            mapping,
            registered: true,
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

    /// Removes the slot from the VM, then unmaps the page.
    ///
    /// The slot is removed and the host copy zeroed even when the guest failed to erase the
    /// page, so the material never outlives this call; the verification result is returned
    /// afterwards.
    pub(crate) fn retire(mut self, vm: &VmFd) -> Result<(), MachineError> {
        let erased = self.is_erased();
        let removal = self.unregister(vm);
        self.mapping.zero(0, LAUNCH_PAGE_SIZE);
        removal?;
        if erased {
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
}
