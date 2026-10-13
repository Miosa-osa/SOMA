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
use super::{
    super::manifest::{PersistenceBinding, STEP_A_PERSISTENCE_PATHS},
    CompilerProfile, GIB, MIB, ProfileLimits,
};

/// The writable class the version 2 machine is defined at, in bytes.
///
/// It is the size the large shape declares as its writable storage, and it is the largest class
/// either machine contract admits. Nothing outside this profile may produce or admit it.
const LARGE_WRITABLE_CLASS_BYTES: u64 = 20 * GIB;

/// The writable class the version 3 default sandbox is defined at, in bytes.
///
/// The default Generation that carries Docker, default egress and the live browser declares ten
/// gigabytes of writable storage, so the version 3 profile names that class; a class the profile
/// does not list can be compiled under a narrowed profile but never certified or admitted.
const DEFAULT_WRITABLE_CLASS_BYTES: u64 = 10 * GIB;

/// The writable class the version 3 xl sandbox is defined at, in bytes.
///
/// The large shape's catalog contract declares forty gigabytes of writable storage and launch
/// refuses a storage size that differs from the manifest, so the version 3 profile names that
/// class too. Version 1 and 2 profiles stay as they were.
const XL_WRITABLE_CLASS_BYTES: u64 = 40 * GIB;

/// The first compiler policy that declares the durable data volume: version 4.
///
/// It is the first policy that does not change the machine. Version 3's machine already has the
/// `vdc` slot in its device vocabulary, so version 4 declares the volume in the Generation's
/// device contract, binds the same machine contract as version 3, and writes manifest group 17.
pub const PERSISTENCE_POLICY_VERSION: u16 = 4;

/// The hotplug region a version 3 profile has when the builder names none: one gigabyte.
pub const DEFAULT_HOTPLUG_BYTES: u64 = GIB;
/// The block a hotplug region is a whole number of: virtio-mem's plug granularity in this
/// machine, which is also the unit the guest onlines.
pub const HOTPLUG_BLOCK_BYTES: u64 = 4 * MIB;
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
            hotplug_bytes: 0,
            // Every profile before the persistence policy builds a Generation with no durable
            // volume, which is what makes those Generations byte for byte the ones they were.
            persistence: PersistenceBinding::default(),
        }
    }

    /// Returns compiler profile version 2 for the multi-vCPU `x86_64` Generation.
    ///
    /// The version 2 machine is the one that carries a large writable class, so the classes this
    /// profile admits are version 1's plus that one. A Generation that declares a class the
    /// profile does not list cannot be compiled or admitted, which is why the class the large
    /// shape is defined at has to be named here rather than only in a test fixture.
    #[must_use]
    pub fn v2() -> Self {
        Self {
            policy_version: 2,
            machine_contract: MachineContract::V2,
            overlay_capacities: vec![256 * MIB, GIB, 4 * GIB, LARGE_WRITABLE_CLASS_BYTES],
            ..Self::v1()
        }
    }

    /// Returns compiler profile version 3: version 2's machine plus a virtio-mem device.
    ///
    /// The kernel it builds must pass the live-resize rules
    /// ([`crate::generation::kernel_config::LIVE_RESIZE_POLICY_VERSION`]), and the manifest binds
    /// machine contract version 3, so a host that does not implement it refuses the Generation
    /// and every version 1 and 2 Generation verifies exactly as before.
    #[must_use]
    pub fn v3() -> Self {
        Self {
            policy_version: 3,
            machine_contract: MachineContract::V3,
            hotplug_bytes: DEFAULT_HOTPLUG_BYTES,
            overlay_capacities: vec![
                256 * MIB,
                GIB,
                4 * GIB,
                DEFAULT_WRITABLE_CLASS_BYTES,
                LARGE_WRITABLE_CLASS_BYTES,
                XL_WRITABLE_CLASS_BYTES,
            ],
            ..Self::v2()
        }
    }

    /// Returns compiler profile version 4: version 3's machine plus the durable data volume.
    ///
    /// The volume is declared by the Generation rather than by a new machine contract, so this
    /// profile binds contract version 3 and the persistence binding is what makes the difference:
    /// the manifest carries group 17, the device set declares slot 6, and the command line names
    /// `vdc`. A host that does not implement the volume slot refuses the Generation, and every
    /// version 1, 2, and 3 Generation verifies exactly as before.
    #[must_use]
    pub fn v4() -> Self {
        Self {
            policy_version: PERSISTENCE_POLICY_VERSION,
            persistence: PersistenceBinding {
                enabled: true,
                paths: STEP_A_PERSISTENCE_PATHS
                    .iter()
                    .map(|path| (*path).to_owned())
                    .collect(),
            },
            ..Self::v3()
        }
    }

    /// This profile with a different hotplug region: the ceiling minus the boot memory.
    ///
    /// Only a profile whose machine has the memory device admits a non-zero region; `validate`
    /// refuses the rest.
    #[must_use]
    pub const fn with_hotplug_bytes(mut self, hotplug_bytes: u64) -> Self {
        self.hotplug_bytes = hotplug_bytes;
        self
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
            // Version 3 states the virtio-mem device, and the machine now builds it on every boot
            // and restore path, so a host admits a Generation built under it.
            3 => Some(Self::v3()),
            PERSISTENCE_POLICY_VERSION => Some(Self::v4()),
            _ => None,
        }
    }

    /// Whether this profile's machine is the one its policy version defines.
    ///
    /// Versions 1 to 3 each define their own machine, so the version and the contract agree
    /// exactly. Version 4 is the first policy that adds a device without changing the machine:
    /// it declares the durable volume in the Generation's device contract and binds version 3's
    /// machine contract, so the equality is stated as the rule it is rather than as a comparison
    /// a later device-only policy would have to break.
    fn machine_is_policy(&self) -> bool {
        match self.policy_version {
            PERSISTENCE_POLICY_VERSION => self.machine_contract == MachineContract::V3,
            version => self.machine_contract.version() == version,
        }
    }

    /// Whether the hotplug region is one this profile's machine can have: none without the
    /// memory device, and a whole number of blocks with it.
    fn hotplug_valid(&self) -> bool {
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
