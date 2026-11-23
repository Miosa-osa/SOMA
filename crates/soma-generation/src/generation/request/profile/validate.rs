//! Check that a compiler profile is one of the certified policies and its hotplug fits.

use super::{
    CompileError, CompileErrorKind, CompilePhase, CompilerProfile, GUEST_CACHE_POLICY_VERSION,
    HOTPLUG_BLOCK_BYTES, LARGE_DISK_POLICY_VERSION, MIB, MachineContract,
    PERSISTENCE_POLICY_VERSION, ProfileLimits, WIDE_CPU_POLICY_VERSION, WIDE_MEMORY_POLICY_VERSION,
};

impl CompilerProfile {
    /// Whether this profile's machine is the one its policy version defines.
    ///
    /// Versions 1 to 3 each define their own machine, so the version and the contract agree
    /// exactly. Version 4 is the first policy that adds a device without changing the machine:
    /// it declares the durable volume in the Generation's device contract and binds version 3's
    /// machine contract, so the equality is stated as the rule it is rather than as a comparison
    /// a later device-only policy would have to break.
    pub(super) fn machine_is_policy(&self) -> bool {
        match self.policy_version {
            PERSISTENCE_POLICY_VERSION => self.machine_contract == MachineContract::V3,
            WIDE_CPU_POLICY_VERSION => self.machine_contract == MachineContract::V4,
            WIDE_MEMORY_POLICY_VERSION | LARGE_DISK_POLICY_VERSION => {
                self.machine_contract == MachineContract::V5
            }
            GUEST_CACHE_POLICY_VERSION => self.machine_contract == MachineContract::V6,
            version => self.machine_contract.version() == version,
        }
    }

    /// Whether the hotplug region is one this profile's machine can have: none without the
    /// memory device, and a whole number of blocks with it.
    pub(super) fn hotplug_valid(&self) -> bool {
        if self.machine_contract.has_memory_device() {
            self.hotplug_bytes > 0 && self.hotplug_bytes.is_multiple_of(HOTPLUG_BLOCK_BYTES)
        } else {
            self.hotplug_bytes == 0
        }
    }

    pub(crate) fn validate(&self) -> Result<(), CompileError> {
        let bounds = self.tree;
        let zero = bounds.max_entries == 0
            || bounds.max_path_bytes == 0
            || bounds.max_link_bytes == 0
            || bounds.max_metadata_bytes == 0
            || bounds.max_file_bytes == 0
            || bounds.max_content_bytes == 0
            || self.max_stream_bytes == 0
            || self.max_root_bytes == 0
            || self.max_kernel_bytes == 0
            || self.max_executable_bytes == 0
            || self.max_initramfs_bytes == 0
            || self.tool_deadline.is_zero();
        let capacities_valid = !self.overlay_capacities.is_empty()
            && self.overlay_capacities.len() <= 16
            && self
                .overlay_capacities
                .windows(2)
                .all(|pair| pair[1] > pair[0])
            && self
                .overlay_capacities
                .iter()
                .all(|capacity| *capacity >= 64 * MIB && capacity.is_multiple_of(4 * MIB));
        let limits_declared = ProfileLimits::for_version(self.policy_version).is_some();
        if zero
            || !limits_declared
            || !self.machine_is_policy()
            || !self.hotplug_valid()
            || !capacities_valid
            || self.guest_agent_provenance.len() > 256
        {
            return Err(CompileError::new(
                CompilePhase::ResolveInputs,
                CompileErrorKind::InvalidInput,
            ));
        }
        Ok(())
    }
}
