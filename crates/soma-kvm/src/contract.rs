//! The versioned `x86_64` machine contract as portable data.
//!
//! A Generation is built under exactly one machine contract and a snapshot is restored against
//! exactly one. Version 1 is the certified single-vCPU machine with a three-gigabyte ceiling.
//! Version 2 admits up to eight vCPUs and sixteen gigabytes, drops `noapic` so the guest
//! programs the I/O APIC, and publishes an Intel MP table because it boots with ACPI off and
//! the MP table is then the only way the kernel finds its application processors.
//!
//! Only the data a host that cannot boot the machine must still be able to compose and verify
//! lives here; the boot machine itself is under the `x86_64` module. This mirrors
//! [`crate::cmdline`], which is portable for the same reason.

use std::fmt;

use crate::mptable;

/// Smallest guest RAM a machine contract admits.
pub const MIN_MEMORY_BYTES: u64 = 128 * 1024 * 1024;
/// Guest RAM granularity every contract fixes.
pub const MEMORY_STEP_BYTES: u64 = 4096;
/// Version number of the single-vCPU contract.
pub const V1_VERSION: u16 = 1;
/// Version number of the multi-vCPU contract.
pub const V2_VERSION: u16 = 2;
/// The one vCPU version 1 admits.
pub const V1_MAX_VCPUS: u16 = 1;
/// The eight vCPUs version 2 admits.
pub const V2_MAX_VCPUS: u16 = mptable::MAX_PROCESSORS;
/// The three-gigabyte guest RAM ceiling version 1 admits.
pub const V1_MAX_MEMORY_BYTES: u64 = 3 * 1024 * 1024 * 1024;
/// The sixteen-gigabyte guest RAM ceiling version 2 admits.
pub const V2_MAX_MEMORY_BYTES: u64 = 16 * 1024 * 1024 * 1024;

/// The fixed ordered diagnostic arguments of machine contract version 1.
pub const V1_BASE_ARGUMENTS: [&str; 9] = [
    "console=ttyS0",
    "reboot=k",
    "panic=1",
    "nomodule",
    "random.trust_cpu=off",
    "pci=off",
    "acpi=off",
    "noapic",
    "cryptomgr.notests",
];

/// The fixed ordered arguments of machine contract version 2.
///
/// Identical to version 1 except that `noapic` is gone: the whole point of version 2 is that
/// the guest brings up its application processors and routes device interrupts through the
/// I/O APIC the MP table describes, and `noapic` would forbid exactly that.
pub const V2_BASE_ARGUMENTS: [&str; 8] = [
    "console=ttyS0",
    "reboot=k",
    "panic=1",
    "nomodule",
    "random.trust_cpu=off",
    "pci=off",
    "acpi=off",
    "cryptomgr.notests",
];

/// One of the machine contracts this implementation understands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MachineContract {
    /// The certified single-vCPU machine.
    V1,
    /// The multi-vCPU machine that discovers processors through an MP table.
    V2,
}

/// A contract version this implementation does not implement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnknownContractVersion {
    /// The version read from the Generation.
    pub version: u16,
}

impl fmt::Display for UnknownContractVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unknown machine contract version {}",
            self.version
        )
    }
}

impl std::error::Error for UnknownContractVersion {}

impl MachineContract {
    /// Every contract this implementation understands, oldest first.
    pub const ALL: [Self; 2] = [Self::V1, Self::V2];

    /// The contract's version number.
    #[must_use]
    pub const fn version(self) -> u16 {
        match self {
            Self::V1 => V1_VERSION,
            Self::V2 => V2_VERSION,
        }
    }

    /// The contract a version number names, or `None` when it is unknown.
    #[must_use]
    pub const fn from_version(version: u16) -> Option<Self> {
        match version {
            V1_VERSION => Some(Self::V1),
            V2_VERSION => Some(Self::V2),
            _ => None,
        }
    }

    /// The contract a version number names, refusing an unknown one.
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
            Self::V1 => V1_MAX_VCPUS,
            Self::V2 => V2_MAX_VCPUS,
        }
    }

    /// The smallest guest RAM this contract admits.
    #[must_use]
    pub const fn min_memory_bytes(self) -> u64 {
        MIN_MEMORY_BYTES
    }

    /// The largest guest RAM this contract admits.
    #[must_use]
    pub const fn max_memory_bytes(self) -> u64 {
        match self {
            Self::V1 => V1_MAX_MEMORY_BYTES,
            Self::V2 => V2_MAX_MEMORY_BYTES,
        }
    }

    /// The guest RAM granularity this contract fixes.
    #[must_use]
    pub const fn memory_step_bytes(self) -> u64 {
        MEMORY_STEP_BYTES
    }

    /// The fixed ordered diagnostic arguments this contract boots with.
    #[must_use]
    pub const fn base_arguments(self) -> &'static [&'static str] {
        match self {
            Self::V1 => &V1_BASE_ARGUMENTS,
            Self::V2 => &V2_BASE_ARGUMENTS,
        }
    }

    /// Whether the machine writes an Intel MP table into guest memory before the guest runs.
    #[must_use]
    pub const fn writes_mp_table(self) -> bool {
        matches!(self, Self::V2)
    }

    /// Whether a vCPU count is one this contract admits.
    #[must_use]
    pub const fn accepts_vcpus(self, vcpus: u16) -> bool {
        vcpus >= 1 && vcpus <= self.max_vcpus()
    }

    /// Whether a guest RAM size is one this contract admits.
    #[must_use]
    pub const fn accepts_memory(self, bytes: u64) -> bool {
        bytes >= MIN_MEMORY_BYTES
            && bytes <= self.max_memory_bytes()
            && bytes.is_multiple_of(MEMORY_STEP_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_round_trip_and_refuse_unknown_ones() {
        for contract in MachineContract::ALL {
            assert_eq!(
                MachineContract::from_version(contract.version()),
                Some(contract)
            );
        }
        assert_eq!(MachineContract::from_version(0), None);
        assert_eq!(MachineContract::from_version(3), None);
        assert_eq!(
            MachineContract::require(9),
            Err(UnknownContractVersion { version: 9 })
        );
    }

    #[test]
    fn version_two_admits_more_than_version_one_and_drops_noapic() {
        assert_eq!(MachineContract::V1.max_vcpus(), 1);
        assert_eq!(MachineContract::V2.max_vcpus(), 8);
        assert_eq!(
            MachineContract::V1.max_memory_bytes(),
            3 * 1024 * 1024 * 1024
        );
        assert_eq!(
            MachineContract::V2.max_memory_bytes(),
            16 * 1024 * 1024 * 1024
        );
        assert!(MachineContract::V1.base_arguments().contains(&"noapic"));
        assert!(!MachineContract::V2.base_arguments().contains(&"noapic"));
        assert!(!MachineContract::V1.writes_mp_table());
        assert!(MachineContract::V2.writes_mp_table());
    }

    #[test]
    fn acceptance_bounds_are_inclusive_and_aligned() {
        let v2 = MachineContract::V2;
        assert!(v2.accepts_vcpus(1));
        assert!(v2.accepts_vcpus(8));
        assert!(!v2.accepts_vcpus(0));
        assert!(!v2.accepts_vcpus(9));
        assert!(v2.accepts_memory(MIN_MEMORY_BYTES));
        assert!(v2.accepts_memory(V2_MAX_MEMORY_BYTES));
        assert!(!v2.accepts_memory(MIN_MEMORY_BYTES + 1));
        assert!(!v2.accepts_memory(V2_MAX_MEMORY_BYTES + MEMORY_STEP_BYTES));
        assert!(!MachineContract::V1.accepts_memory(V1_MAX_MEMORY_BYTES + MEMORY_STEP_BYTES));
    }
}
