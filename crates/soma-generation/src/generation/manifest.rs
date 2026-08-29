use soma::OciPlatform;

use super::{
    artifacts::{ArtifactDescriptor, Sha256Digest},
    contracts::ContractBinding,
};

mod decode;
mod encode;

pub use decode::decode_manifest;
pub use encode::encode_manifest;

/// The `SOMAGEN` manifest schema version produced and accepted by this module.
pub const MANIFEST_SCHEMA_VERSION: u16 = 1;
/// Maximum encoded manifest size.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub(crate) const MAGIC: &[u8; 8] = b"SOMAGEN\0";
pub(crate) const MAX_SHORT_STRING: usize = 256;
pub(crate) const MAX_COMMAND_LINE: usize = 8191;
pub(crate) const MAX_TEMPLATES: usize = 16;

/// Group 2: the source OCI identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceBinding {
    /// The selected OCI manifest digest.
    pub oci_manifest_digest: Sha256Digest,
    /// The effective OCI platform, which must be `linux/amd64` for profile v1.
    pub platform: OciPlatform,
}

/// Group 3: the normalized-tree identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TreeBinding {
    /// The canonical tree-manifest digest.
    pub digest: Sha256Digest,
    /// The exact tree-manifest byte length.
    pub size: u64,
}

/// Group 4: the immutable EROFS root image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RootBinding {
    /// The EROFS image descriptor.
    pub descriptor: ArtifactDescriptor,
    /// The filesystem UUID derived from the tree digest.
    pub uuid: [u8; 16],
    /// The immutable format-profile name.
    pub format_profile: String,
    /// The digest of the formatter executable that produced the image.
    pub formatter_digest: Sha256Digest,
    /// The formatter revision string.
    pub formatter_revision: String,
    /// The content-addressed builder image, or `None` for a host toolchain build.
    pub builder_image_digest: Option<Sha256Digest>,
}

/// One sterile overlay template for an exact writable capacity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverlayTemplate {
    /// The exact writable capacity in bytes.
    pub capacity: u64,
    /// The template image descriptor.
    pub descriptor: ArtifactDescriptor,
}

/// Group 5: the sterile ext4 overlay contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverlayBinding {
    /// The UUID and hash-seed derivation policy version.
    pub uuid_derivation_version: u16,
    /// The canonical feature-profile string.
    pub feature_profile: String,
    /// The minimum supported writable capacity.
    pub minimum_capacity: u64,
    /// The maximum supported writable capacity.
    pub maximum_capacity: u64,
    /// The compiled templates in strictly ascending capacity order.
    pub templates: Vec<OverlayTemplate>,
}

/// Group 6: the kernel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelBinding {
    /// The kernel image descriptor.
    pub descriptor: ArtifactDescriptor,
    /// The ELF and PVH contract version.
    pub elf_pvh_contract_version: u16,
    /// The digest of the kernel configuration text.
    pub config_digest: Sha256Digest,
    /// The CPU architecture name.
    pub cpu_architecture: String,
}

/// Group 7: the initramfs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InitramfsBinding {
    /// The archive descriptor.
    pub descriptor: ArtifactDescriptor,
    /// The initramfs layout version.
    pub layout_version: u16,
    /// The digest of the early-init executable inside the archive.
    pub early_init_digest: Sha256Digest,
}

/// Group 8: the guest agent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuestAgentBinding {
    /// The executable descriptor.
    pub descriptor: ArtifactDescriptor,
    /// The bounded build-provenance string.
    pub build_provenance: String,
    /// The application protocol version.
    pub application_protocol_version: u16,
    /// The handshake protocol version.
    pub handshake_protocol_version: u16,
}

/// Group 13: the exact machine shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MachineShape {
    /// Guest memory in bytes.
    pub memory_bytes: u64,
    /// The vCPU count.
    pub vcpu_count: u16,
    /// The memory-slot layout version.
    pub memory_slot_layout_version: u16,
    /// The immutable launch-page layout version.
    pub launch_page_layout_version: u16,
}

/// Group 14: the certified snapshot, or its typed absence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotBinding {
    /// No snapshot has been captured; the Generation is not launchable.
    Absent,
    /// A captured snapshot pair.
    Captured {
        /// The snapshot format version.
        format_version: u16,
        /// The memory image descriptor.
        memory: ArtifactDescriptor,
        /// The state manifest descriptor.
        state: ArtifactDescriptor,
        /// The capture-point version.
        capture_point_version: u16,
    },
}

/// Group 15: the repair policy and readiness command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RepairBinding {
    /// The required repair-policy version.
    pub policy_version: u16,
    /// The digest of the fixed readiness command.
    pub readiness_command_digest: Sha256Digest,
}

/// The complete canonical `SOMAGEN` v1 manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenerationManifest {
    /// Group 1: the compiler-policy version paired with the schema version.
    pub compiler_policy_version: u16,
    /// Group 2.
    pub source: SourceBinding,
    /// Group 3.
    pub tree: TreeBinding,
    /// Group 4.
    pub root: RootBinding,
    /// Group 5.
    pub overlay: OverlayBinding,
    /// Group 6.
    pub kernel: KernelBinding,
    /// Group 7.
    pub initramfs: InitramfsBinding,
    /// Group 8.
    pub guest_agent: GuestAgentBinding,
    /// Group 9: the complete kernel command line bytes.
    pub command_line: Vec<u8>,
    /// Group 10.
    pub machine_contract: ContractBinding,
    /// Group 11.
    pub device_contract: ContractBinding,
    /// Group 12.
    pub cpu_template: ContractBinding,
    /// Group 13.
    pub shape: MachineShape,
    /// Group 14.
    pub snapshot: SnapshotBinding,
    /// Group 15.
    pub repair: RepairBinding,
}

impl GenerationManifest {
    /// Returns every artifact descriptor in manifest order.
    #[must_use]
    pub fn descriptors(&self) -> Vec<ArtifactDescriptor> {
        let mut descriptors = vec![self.root.descriptor];
        descriptors.extend(
            self.overlay
                .templates
                .iter()
                .map(|template| template.descriptor),
        );
        descriptors.push(self.kernel.descriptor);
        descriptors.push(self.initramfs.descriptor);
        descriptors.push(self.guest_agent.descriptor);
        if let SnapshotBinding::Captured { memory, state, .. } = self.snapshot {
            descriptors.push(memory);
            descriptors.push(state);
        }
        descriptors
    }
}
