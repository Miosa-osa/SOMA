//! The compiler profiles this build implements, and what each one means.
//!
//! A profile is the versioned, fully explicit description of one compiler policy. The
//! constructors live here rather than beside the type so that the list of versions this build
//! implements is one file an operator can read, and so that resolving a version a Generation
//! declares is a lookup with an answer for every version rather than a set of callers each
//! naming the one they assume.

use std::time::Duration;

use soma_kvm::MachineContract;

use super::super::{
    error::{CompileError, CompileErrorKind, CompilePhase},
    tree_decoder::TreeBounds,
};
use super::{CompilerProfile, GIB, MIB, ProfileLimits};
impl CompilerProfile {
    /// Returns compiler profile version 1 for the `x86_64` EROFS-plus-overlay Generation.
    #[must_use]
    pub fn v1() -> Self {
        Self {
            policy_version: 1,
            epoch: 1_700_000_000,
            tree: TreeBounds {
                max_entries: 1_000_000,
                max_path_bytes: 4_096,
                max_link_bytes: 4_096,
                max_metadata_bytes: 64 * MIB,
                max_file_bytes: 8 * GIB,
                max_content_bytes: 128 * GIB,
            },
            max_stream_bytes: 160 * GIB,
            max_root_bytes: 160 * GIB,
            max_kernel_bytes: 64 * MIB,
            max_executable_bytes: 64 * MIB,
            max_initramfs_bytes: 128 * MIB,
            tool_deadline: Duration::from_secs(3_600),
            overlay_capacities: vec![256 * MIB, GIB, 4 * GIB],
            guest_agent_provenance: "soma-guest-agent:unpinned-development-input".to_owned(),
            application_protocol_version: 1,
            handshake_protocol_version: 1,
            machine_contract: MachineContract::V1,
        }
    }

    /// Returns compiler profile version 2 for the multi-vCPU `x86_64` Generation.
    #[must_use]
    pub fn v2() -> Self {
        Self {
            policy_version: 2,
            machine_contract: MachineContract::V2,
            ..Self::v1()
        }
    }

    /// Returns the compiler profile one compiler-policy version names, or `None` for a version
    /// this build does not implement.
    ///
    /// A Generation states the policy it was built under, and the profile is what that version
    /// means. Resolving the profile from the version a Generation declares is what lets a host
    /// admit a Generation built under a newer machine contract without being told in advance
    /// which contract that is; a version with no profile here is refused rather than admitted
    /// under the wrong one.
    #[must_use]
    pub fn from_policy_version(version: u16) -> Option<Self> {
        match version {
            1 => Some(Self::v1()),
            2 => Some(Self::v2()),
            _ => None,
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
            || self.machine_contract.version() != self.policy_version
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
