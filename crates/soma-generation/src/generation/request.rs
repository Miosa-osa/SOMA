use std::{fmt, path::Path, time::Duration};

use super::{
    error::{CompileError, CompileErrorKind, CompilePhase},
    template::TemplateRevision,
    tree_decoder::TreeBounds,
};
use crate::NormalizedRootfs;

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// The versioned, fully explicit compiler profile.
///
/// It contains no registry credential, cloud identifier, host path, current time, random seed,
/// or shell fragment; every value is a bound or a pinned policy constant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompilerProfile {
    /// The compiler-policy version bound into the manifest.
    pub policy_version: u16,
    /// The fixed filesystem and build epoch applied to every immutable artifact.
    pub epoch: u64,
    /// Bounds for decoding the canonical tree.
    pub tree: TreeBounds,
    /// Maximum bytes of the emitted tar stream.
    pub max_stream_bytes: u64,
    /// Maximum bytes of the produced EROFS image.
    pub max_root_bytes: u64,
    /// Maximum bytes of the kernel ELF image.
    pub max_kernel_bytes: u64,
    /// Maximum bytes of one early-init or guest-agent executable.
    pub max_executable_bytes: u64,
    /// Maximum bytes of the produced initramfs.
    pub max_initramfs_bytes: u64,
    /// Wall-clock limit for one pinned tool invocation.
    pub tool_deadline: Duration,
    /// Certified sterile overlay-template size classes in strictly ascending order.
    ///
    /// A Template revision selects exactly one of these as its writable-storage size class.
    pub overlay_capacities: Vec<u64>,
    /// The bounded guest-agent build-provenance string.
    pub guest_agent_provenance: String,
    /// The guest application protocol version.
    pub application_protocol_version: u16,
    /// The guest handshake protocol version.
    pub handshake_protocol_version: u16,
}

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
        if zero
            || self.policy_version != 1
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

/// Directories holding the pinned external tools.
///
/// These are compiler configuration and never enter an artifact or a manifest.
#[derive(Clone, Copy)]
pub struct Toolchain<'a> {
    pub(crate) erofs_utils: &'a Path,
    pub(crate) e2fsprogs: &'a Path,
}

impl<'a> Toolchain<'a> {
    /// Names the directories that contain `mkfs.erofs`, `fsck.erofs`, `mke2fs`, `e2fsck`,
    /// `debugfs`, and `dumpe2fs`.
    #[must_use]
    pub const fn new(erofs_utils: &'a Path, e2fsprogs: &'a Path) -> Self {
        Self {
            erofs_utils,
            e2fsprogs,
        }
    }
}

impl fmt::Debug for Toolchain<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Toolchain")
            .field("erofs_utils", &"[REDACTED]")
            .field("e2fsprogs", &"[REDACTED]")
            .finish()
    }
}

/// Files supplying the pinned kernel, configuration, early init, guest agent, and the
/// Generation-scoped responder private key that the initramfs carries to the guest agent.
#[derive(Clone, Copy)]
pub struct MachineInputs<'a> {
    pub(crate) kernel: &'a Path,
    pub(crate) kernel_config: &'a Path,
    pub(crate) early_init: &'a Path,
    pub(crate) guest_agent: &'a Path,
    pub(crate) responder_key: &'a Path,
}

impl<'a> MachineInputs<'a> {
    /// Names the five machine input files; `responder_key` holds exactly 32 raw key bytes.
    #[must_use]
    pub const fn new(
        kernel: &'a Path,
        kernel_config: &'a Path,
        early_init: &'a Path,
        guest_agent: &'a Path,
        responder_key: &'a Path,
    ) -> Self {
        Self {
            kernel,
            kernel_config,
            early_init,
            guest_agent,
            responder_key,
        }
    }
}

impl fmt::Debug for MachineInputs<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MachineInputs")
            .field("kernel", &"[REDACTED]")
            .field("kernel_config", &"[REDACTED]")
            .field("early_init", &"[REDACTED]")
            .field("guest_agent", &"[REDACTED]")
            .field("responder_key", &"[REDACTED]")
            .finish()
    }
}

/// Host-only build resources: the staging directory, the pinned tools, and the machine inputs.
///
/// None of these locations enters an artifact or a manifest.
#[derive(Clone, Copy)]
pub struct BuildHost<'a> {
    pub(crate) staging: &'a Path,
    pub(crate) toolchain: Toolchain<'a>,
    pub(crate) inputs: MachineInputs<'a>,
}

impl<'a> BuildHost<'a> {
    /// Names the host resources.
    ///
    /// `staging` names an existing private directory on one bounded writable volume; the
    /// compiler creates and removes a unique subdirectory inside it for formatter output.
    #[must_use]
    pub const fn new(
        staging: &'a Path,
        toolchain: Toolchain<'a>,
        inputs: MachineInputs<'a>,
    ) -> Self {
        Self {
            staging,
            toolchain,
            inputs,
        }
    }
}

impl fmt::Debug for BuildHost<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BuildHost")
            .field("staging", &"[REDACTED]")
            .field("toolchain", &self.toolchain)
            .field("inputs", &self.inputs)
            .finish()
    }
}

/// One explicit Generation compilation request: one Template revision, its normalized tree,
/// the content store, the versioned profile, and host-only build resources.
#[derive(Clone, Copy)]
pub struct CompileGeneration<'a> {
    pub(crate) template: &'a TemplateRevision,
    pub(crate) normalized: &'a NormalizedRootfs,
    pub(crate) store: &'a Path,
    pub(crate) profile: &'a CompilerProfile,
    pub(crate) host: BuildHost<'a>,
}

impl<'a> CompileGeneration<'a> {
    /// Creates a request for an existing normalized tree in an existing content store.
    #[must_use]
    pub const fn new(
        template: &'a TemplateRevision,
        normalized: &'a NormalizedRootfs,
        store: &'a Path,
        profile: &'a CompilerProfile,
        host: BuildHost<'a>,
    ) -> Self {
        Self {
            template,
            normalized,
            store,
            profile,
            host,
        }
    }
}

impl fmt::Debug for CompileGeneration<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompileGeneration")
            .field("template", &self.template)
            .field("normalized", &self.normalized)
            .field("store", &"[REDACTED]")
            .field("profile", &self.profile)
            .field("host", &self.host)
            .finish()
    }
}
