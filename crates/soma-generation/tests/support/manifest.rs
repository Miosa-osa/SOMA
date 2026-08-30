#![allow(dead_code)]

use soma::{NetworkPolicy, OciPlatform};
use soma_generation::{
    ArtifactDescriptor, ArtifactRole, Sha256Digest, contracts,
    generation_manifest::{
        GenerationManifest, GuestAgentBinding, InitramfsBinding, KernelBinding,
        MachineShapeBinding, OverlayBinding, OverlayTemplate, RepairBinding, RootBinding,
        SnapshotBinding, SourceBinding, TemplateBinding, TreeBinding,
    },
    template::{NetworkPolicyClass, network_policy_digest},
};

pub fn digest(fill: u8) -> Sha256Digest {
    Sha256Digest::from_bytes([fill; 32])
}

pub fn descriptor(role: ArtifactRole, fill: u8, size: u64) -> ArtifactDescriptor {
    ArtifactDescriptor {
        role,
        digest: digest(fill),
        size,
    }
}

pub fn sample() -> GenerationManifest {
    GenerationManifest {
        compiler_policy_version: 1,
        source: SourceBinding {
            oci_manifest_digest: digest(0x01),
            platform: OciPlatform::linux_amd64(),
        },
        tree: TreeBinding {
            digest: digest(0x02),
            size: 3_678_098,
        },
        root: RootBinding {
            descriptor: descriptor(ArtifactRole::ErofsRoot, 0x03, 1_200_000_000),
            uuid: [0x10; 16],
            format_profile: "erofs/v1/blk4096/uncompressed/no-xattr/tar-full/all-time".to_owned(),
            formatter_digest: digest(0x04),
            formatter_revision: "1.9.4".to_owned(),
            builder_environment_digest: digest(0x05),
        },
        overlay: OverlayBinding {
            uuid_derivation_version: 1,
            feature_profile: "ext4/test".to_owned(),
            minimum_capacity: 1 << 26,
            maximum_capacity: 1 << 27,
            templates: vec![
                OverlayTemplate {
                    capacity: 1 << 26,
                    descriptor: descriptor(ArtifactRole::OverlayTemplate, 0x05, 1 << 26),
                },
                OverlayTemplate {
                    capacity: 1 << 27,
                    descriptor: descriptor(ArtifactRole::OverlayTemplate, 0x06, 1 << 27),
                },
            ],
        },
        kernel: KernelBinding {
            descriptor: descriptor(ArtifactRole::Kernel, 0x07, 9_000_000),
            elf_pvh_contract_version: 1,
            config_digest: digest(0x08),
            cpu_architecture: "x86_64".to_owned(),
        },
        initramfs: InitramfsBinding {
            descriptor: descriptor(ArtifactRole::Initramfs, 0x09, 2_000_000),
            layout_version: 1,
            early_init_digest: digest(0x0a),
        },
        guest_agent: GuestAgentBinding {
            descriptor: descriptor(ArtifactRole::GuestAgent, 0x0b, 1_500_000),
            build_provenance: "test".to_owned(),
            application_protocol_version: 1,
            handshake_protocol_version: 1,
        },
        command_line: contracts::kernel_command_line_v1(),
        machine_contract: contracts::machine_contract_v1(),
        device_contract: contracts::device_contract_v1(),
        cpu_template: contracts::cpu_template_v1(),
        shape: MachineShapeBinding {
            memory_bytes: 512 << 20,
            vcpu_count: 1,
            memory_slot_layout_version: 1,
            launch_page_layout_version: 1,
        },
        snapshot: SnapshotBinding::Absent,
        repair: RepairBinding {
            policy_version: 1,
            readiness_command_digest: contracts::readiness_command_digest(),
        },
        template: TemplateBinding {
            writable_storage_bytes: 1 << 26,
            network_policy_class: NetworkPolicyClass::Isolated,
            network_policy_digest: network_policy_digest(&NetworkPolicy::isolated()).unwrap(),
            workload_probe: None,
            ttl_seconds: 600,
        },
    }
}
