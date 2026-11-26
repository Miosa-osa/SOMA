//! The UEFI machine contract family as portable data (ADR 0068).
//!
//! A UEFI machine boots firmware from a disk image instead of a kernel by PVH entry: ACPI on,
//! virtio block and network on PCI, a TPM 2.0, one serial port and the Hyper-V enlightenments,
//! with no kernel command line. Its versions start at 1001 so they never share a number with the
//! PVH [`MachineContract`] line, and every PVH boot, snapshot and restore path, which asks
//! [`MachineContract::from_version`], refuses a UEFI version as unknown.

use super::{MEMORY_STEP_BYTES, MachineContract, UnknownContractVersion};

/// Version number of the first UEFI contract.
pub const UEFI_V1_VERSION: u16 = 1001;
/// The largest vCPU count UEFI version 1 admits.
pub const UEFI_V1_MAX_VCPUS: u16 = 32;
/// The smallest guest RAM UEFI version 1 admits: firmware and a desktop operating system.
pub const UEFI_V1_MIN_MEMORY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// The largest guest RAM UEFI version 1 admits.
pub const UEFI_V1_MAX_MEMORY_BYTES: u64 = 64 * 1024 * 1024 * 1024;

/// How a machine contract starts its guest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Firmware {
    /// Direct kernel entry by PVH: the [`MachineContract`] family.
    Pvh,
    /// UEFI firmware that boots a disk image: the [`UefiContract`] family.
    Uefi,
}

impl Firmware {
    /// The family a contract version belongs to, or `None` when no family has that version.
    #[must_use]
    pub const fn of_contract_version(version: u16) -> Option<Self> {
        if MachineContract::from_version(version).is_some() {
            Some(Self::Pvh)
        } else if UefiContract::from_version(version).is_some() {
            Some(Self::Uefi)
        } else {
            None
        }
    }
}

/// A versioned UEFI machine contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UefiContract {
    /// UEFI firmware, ACPI, virtio block and network on PCI, a TPM 2.0, one serial port and the
    /// Hyper-V enlightenments.
    V1,
}

impl UefiContract {
    /// Every UEFI contract this implementation builds, oldest first.
    pub const ALL: [Self; 1] = [Self::V1];

    /// The contract's version number.
    #[must_use]
    pub const fn version(self) -> u16 {
        match self {
            Self::V1 => UEFI_V1_VERSION,
        }
    }

    /// The contract a version number names, or `None` when it is not a UEFI version.
    #[must_use]
    pub const fn from_version(version: u16) -> Option<Self> {
        match version {
            UEFI_V1_VERSION => Some(Self::V1),
            _ => None,
        }
    }

    /// The contract a version number names, refusing one that is not a UEFI version.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownContractVersion`] for a version this implementation does not build.
    pub const fn require(version: u16) -> Result<Self, UnknownContractVersion> {
        match Self::from_version(version) {
            Some(contract) => Ok(contract),
            None => Err(UnknownContractVersion { version }),
        }
    }

    /// The largest vCPU count this contract admits.
    #[must_use]
    pub const fn max_vcpus(self) -> u16 {
        match self {
            Self::V1 => UEFI_V1_MAX_VCPUS,
        }
    }

    /// The smallest guest RAM this contract admits.
    #[must_use]
    pub const fn min_memory_bytes(self) -> u64 {
        match self {
            Self::V1 => UEFI_V1_MIN_MEMORY_BYTES,
        }
    }

    /// The largest guest RAM this contract admits.
    #[must_use]
    pub const fn max_memory_bytes(self) -> u64 {
        match self {
            Self::V1 => UEFI_V1_MAX_MEMORY_BYTES,
        }
    }

    /// Whether a vCPU count is one this contract admits.
    #[must_use]
    pub const fn accepts_vcpus(self, vcpus: u16) -> bool {
        vcpus >= 1 && vcpus <= self.max_vcpus()
    }

    /// Whether a guest RAM size is one this contract admits.
    #[must_use]
    pub const fn accepts_memory(self, bytes: u64) -> bool {
        bytes >= self.min_memory_bytes()
            && bytes <= self.max_memory_bytes()
            && bytes.is_multiple_of(MEMORY_STEP_BYTES)
    }

    /// Whether the guest sees the Hyper-V enlightenments, which Windows needs to boot.
    #[must_use]
    pub const fn hyperv_enlightenments(self) -> bool {
        true
    }

    /// Whether the machine has a TPM 2.0, backed by one software TPM per Instance.
    #[must_use]
    pub const fn has_tpm(self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uefi_versions_round_trip_and_never_name_a_pvh_contract() {
        for contract in UefiContract::ALL {
            assert_eq!(
                UefiContract::from_version(contract.version()),
                Some(contract)
            );
            assert_eq!(UefiContract::require(contract.version()), Ok(contract));
            assert_eq!(MachineContract::from_version(contract.version()), None);
        }
        assert_eq!(
            UefiContract::require(6),
            Err(UnknownContractVersion { version: 6 })
        );
    }

    #[test]
    fn firmware_names_the_family_of_every_known_version_and_refuses_others() {
        for contract in MachineContract::ALL {
            assert_eq!(
                Firmware::of_contract_version(contract.version()),
                Some(Firmware::Pvh)
            );
        }
        for contract in UefiContract::ALL {
            assert_eq!(
                Firmware::of_contract_version(contract.version()),
                Some(Firmware::Uefi)
            );
        }
        for unknown in [0, 7, 1000, 1002, u16::MAX] {
            assert_eq!(
                Firmware::of_contract_version(unknown),
                None,
                "version {unknown}"
            );
        }
    }

    #[test]
    fn uefi_v1_shape_bounds() {
        let contract = UefiContract::V1;
        assert!(contract.accepts_vcpus(1) && contract.accepts_vcpus(UEFI_V1_MAX_VCPUS));
        assert!(!contract.accepts_vcpus(0) && !contract.accepts_vcpus(UEFI_V1_MAX_VCPUS + 1));
        assert!(contract.accepts_memory(UEFI_V1_MIN_MEMORY_BYTES));
        assert!(contract.accepts_memory(8 * 1024 * 1024 * 1024));
        assert!(!contract.accepts_memory(UEFI_V1_MIN_MEMORY_BYTES - MEMORY_STEP_BYTES));
        assert!(!contract.accepts_memory(UEFI_V1_MAX_MEMORY_BYTES + MEMORY_STEP_BYTES));
        assert!(!contract.accepts_memory(UEFI_V1_MIN_MEMORY_BYTES + 1));
        assert!(contract.hyperv_enlightenments() && contract.has_tpm());
    }
}
