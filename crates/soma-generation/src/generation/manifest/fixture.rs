//! One canonical manifest the crate's own tests mutate field by field.
//!
//! The values match compiler profile v1 so a test only has to change the field it is about.

use soma::{NetworkPolicy, OciPlatform};

use super::{
    GenerationManifest, GuestAgentBinding, InitramfsBinding, KernelBinding, MachineShapeBinding,
    OverlayBinding, OverlayTemplate, RepairBinding, RootBinding, SnapshotBinding, SourceBinding,
    TemplateBinding, TreeBinding,
};
use crate::generation::{
    artifacts::{ArtifactDescriptor, ArtifactRole, Sha256Digest},
    contracts,
    erofs::{self, derive_root_uuid},
    initramfs::INITRAMFS_LAYOUT_VERSION,
    kernel::ELF_PVH_CONTRACT_VERSION,
    overlay::{OVERLAY_UUID_DERIVATION_VERSION, overlay_feature_profile},
    template::{NetworkPolicyClass, network_policy_digest},
};

const MIB: u64 = 1024 * 1024;

pub(crate) fn digest(fill: u8) -> Sha256Digest {
    Sha256Digest::from_bytes([fill; 32])
}

pub(crate) fn descriptor(role: ArtifactRole, fill: u8, size: u64) -> ArtifactDescriptor {
    ArtifactDescriptor {
        role,
        digest: digest(fill),
        size,
    }
}

/// A manifest that satisfies every profile v1 compatibility rule.
pub(crate) fn profile_v1() -> GenerationManifest {
    let tree = TreeBinding {
        digest: digest(0x02),
        size: 3_678_098,
    };
    GenerationManifest {
        compiler_policy_version: 1,
        source: SourceBinding {
            oci_manifest_digest: digest(0x01),
            platform: OciPlatform::linux_amd64(),
        },
        tree,
        root: RootBinding {
            descriptor: descriptor(ArtifactRole::ErofsRoot, 0x03, 1_200_001_024),
            uuid: derive_root_uuid(&tree.digest),
            format_profile: erofs::EROFS_FORMAT_PROFILE.to_owned(),
            formatter_digest: digest(0x04),
            formatter_revision: erofs::EROFS_UTILS_REVISION.to_owned(),
            builder_image_digest: None,
        },
        overlay: OverlayBinding {
            uuid_derivation_version: OVERLAY_UUID_DERIVATION_VERSION,
            feature_profile: overlay_feature_profile(),
            minimum_capacity: 256 * MIB,
            maximum_capacity: 1024 * MIB,
            templates: vec![
                OverlayTemplate {
                    capacity: 256 * MIB,
                    descriptor: descriptor(ArtifactRole::OverlayTemplate, 0x05, 256 * MIB),
                },
                OverlayTemplate {
                    capacity: 1024 * MIB,
                    descriptor: descriptor(ArtifactRole::OverlayTemplate, 0x06, 1024 * MIB),
                },
            ],
        },
        kernel: KernelBinding {
            descriptor: descriptor(ArtifactRole::Kernel, 0x07, 9_000_000),
            elf_pvh_contract_version: ELF_PVH_CONTRACT_VERSION,
            config_digest: digest(0x08),
            cpu_architecture: "x86_64".to_owned(),
        },
        initramfs: InitramfsBinding {
            descriptor: descriptor(ArtifactRole::Initramfs, 0x09, 2_000_000),
            layout_version: INITRAMFS_LAYOUT_VERSION,
            early_init_digest: digest(0x0a),
        },
        guest_agent: GuestAgentBinding {
            descriptor: descriptor(ArtifactRole::GuestAgent, 0x0b, 1_500_000),
            build_provenance: "soma-guest-agent:test".to_owned(),
            application_protocol_version: 1,
            handshake_protocol_version: 1,
        },
        command_line: contracts::kernel_command_line_v1(),
        machine_contract: contracts::machine_contract_v1(),
        device_contract: contracts::device_contract_v1(),
        cpu_template: contracts::cpu_template_v1(),
        shape: MachineShapeBinding {
            memory_bytes: 512 * MIB,
            vcpu_count: 1,
            memory_slot_layout_version: contracts::MEMORY_SLOT_LAYOUT_VERSION,
            launch_page_layout_version: contracts::LAUNCH_PAGE_LAYOUT_VERSION,
        },
        snapshot: SnapshotBinding::Absent,
        repair: RepairBinding {
            policy_version: contracts::REPAIR_POLICY_VERSION,
            readiness_command_digest: contracts::readiness_command_digest(),
        },
        template: TemplateBinding {
            writable_storage_bytes: 256 * MIB,
            network_policy_class: NetworkPolicyClass::Isolated,
            network_policy_digest: network_policy_digest(&NetworkPolicy::isolated())
                .expect("the isolated policy serializes"),
            workload_probe: None,
            ttl_seconds: 3600,
        },
    }
}

/// A captured snapshot binding a certification token can carry.
pub(crate) fn captured_snapshot() -> SnapshotBinding {
    SnapshotBinding::Captured {
        format_version: contracts::SNAPSHOT_FORMAT_VERSION,
        memory: descriptor(ArtifactRole::MemorySnapshot, 0x20, 512 * MIB),
        state: descriptor(ArtifactRole::StateManifest, 0x21, 4096),
        capture_point_version: contracts::SNAPSHOT_CAPTURE_POINT_VERSION,
    }
}
