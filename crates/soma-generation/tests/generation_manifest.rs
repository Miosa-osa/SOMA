mod support;

use soma::OciPlatform;
use soma_generation::{
    ArtifactDescriptor, ArtifactRole, CompileErrorKind, Sha256Digest, contracts,
    derive_generation_id,
    generation_manifest::{
        GenerationManifest, GuestAgentBinding, InitramfsBinding, KernelBinding, MachineShape,
        OverlayBinding, OverlayTemplate, RepairBinding, RootBinding, SnapshotBinding,
        SourceBinding, TreeBinding, decode_manifest, encode_manifest,
    },
};

const GOLDEN_HEX: &str = include_str!("fixtures/somagen_v1.hex");
const GOLDEN_ID: &str = "sha256:67b55520b5966ee58399db018dfbe71443f55f013f8371b831237988eacdb04e";

fn digest(fill: u8) -> Sha256Digest {
    Sha256Digest::from_bytes([fill; 32])
}

fn descriptor(role: ArtifactRole, fill: u8, size: u64) -> ArtifactDescriptor {
    ArtifactDescriptor {
        role,
        digest: digest(fill),
        size,
    }
}

fn sample() -> GenerationManifest {
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
            builder_image_digest: None,
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
        shape: MachineShape {
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
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut output, byte| {
        write!(output, "{byte:02x}").unwrap();
        output
    })
}

type Mutation = (&'static str, fn(&mut GenerationManifest));

fn artifact_mutations() -> Vec<Mutation> {
    vec![
        ("policy", |m| m.compiler_policy_version = 2),
        ("oci manifest", |m| {
            m.source.oci_manifest_digest = digest(0xa1);
        }),
        ("tree digest", |m| m.tree.digest = digest(0xa2)),
        ("tree size", |m| m.tree.size += 1),
        ("root digest", |m| m.root.descriptor.digest = digest(0xa3)),
        ("root size", |m| m.root.descriptor.size += 1),
        ("root uuid", |m| m.root.uuid[0] ^= 1),
        ("format profile", |m| m.root.format_profile.push('x')),
        ("formatter digest", |m| {
            m.root.formatter_digest = digest(0xa4);
        }),
        ("formatter revision", |m| {
            "1.9.5".clone_into(&mut m.root.formatter_revision);
        }),
        ("builder image", |m| {
            m.root.builder_image_digest = Some(digest(0xa5));
        }),
        ("overlay derivation", |m| {
            m.overlay.uuid_derivation_version = 2;
        }),
        ("overlay features", |m| m.overlay.feature_profile.push('x')),
        ("overlay minimum", |m| m.overlay.minimum_capacity += 1),
        ("overlay maximum", |m| m.overlay.maximum_capacity += 1),
        ("overlay template", |m| {
            m.overlay.templates[0].descriptor.digest = digest(0xa6);
        }),
        ("overlay count", |m| {
            m.overlay.templates.pop();
        }),
        ("kernel digest", |m| {
            m.kernel.descriptor.digest = digest(0xa7);
        }),
        ("kernel contract", |m| m.kernel.elf_pvh_contract_version = 2),
        ("kernel config", |m| m.kernel.config_digest = digest(0xa8)),
        ("kernel arch", |m| {
            "x86-64".clone_into(&mut m.kernel.cpu_architecture);
        }),
        ("initramfs digest", |m| {
            m.initramfs.descriptor.digest = digest(0xa9);
        }),
        ("initramfs layout", |m| m.initramfs.layout_version = 2),
        ("early init", |m| {
            m.initramfs.early_init_digest = digest(0xaa);
        }),
        ("agent digest", |m| {
            m.guest_agent.descriptor.digest = digest(0xab);
        }),
        ("agent provenance", |m| {
            m.guest_agent.build_provenance.push('x');
        }),
        ("agent app protocol", |m| {
            m.guest_agent.application_protocol_version = 2;
        }),
        ("agent handshake", |m| {
            m.guest_agent.handshake_protocol_version = 2;
        }),
    ]
}

fn machine_mutations() -> Vec<Mutation> {
    vec![
        ("command line", |m| m.command_line.push(b'x')),
        ("machine contract", |m| {
            m.machine_contract.digest = digest(0xac);
        }),
        ("machine version", |m| m.machine_contract.version = 2),
        ("device contract", |m| {
            m.device_contract.digest = digest(0xad);
        }),
        ("cpu template", |m| m.cpu_template.digest = digest(0xae)),
        ("memory", |m| m.shape.memory_bytes += 4096),
        ("vcpu", |m| m.shape.vcpu_count = 2),
        ("slot layout", |m| m.shape.memory_slot_layout_version = 2),
        ("launch page", |m| m.shape.launch_page_layout_version = 2),
        ("snapshot", |m| {
            m.snapshot = SnapshotBinding::Captured {
                format_version: 1,
                memory: descriptor(ArtifactRole::MemorySnapshot, 0xaf, 1),
                state: descriptor(ArtifactRole::StateManifest, 0xb0, 1),
                capture_point_version: 1,
            }
        }),
        ("repair policy", |m| m.repair.policy_version = 2),
        ("readiness command", |m| {
            m.repair.readiness_command_digest = digest(0xb1);
        }),
    ]
}

#[test]
fn manifest_has_pinned_golden_bytes_and_identity() {
    let bytes = encode_manifest(&sample()).unwrap();
    assert_eq!(
        hex(&bytes),
        GOLDEN_HEX.trim(),
        "golden mismatch; actual hex follows\n{}",
        hex(&bytes)
    );
    assert_eq!(derive_generation_id(&bytes).as_str(), GOLDEN_ID);
    assert!(bytes.starts_with(b"SOMAGEN\0\x00\x01"));
}

#[test]
fn manifest_round_trips_through_the_hostile_decoder() {
    let manifest = sample();
    let bytes = encode_manifest(&manifest).unwrap();
    assert_eq!(decode_manifest(&bytes).unwrap(), manifest);
    let captured = GenerationManifest {
        snapshot: SnapshotBinding::Captured {
            format_version: 1,
            memory: descriptor(ArtifactRole::MemorySnapshot, 0x0c, 512 << 20),
            state: descriptor(ArtifactRole::StateManifest, 0x0d, 4096),
            capture_point_version: 1,
        },
        ..manifest
    };
    let bytes = encode_manifest(&captured).unwrap();
    assert_eq!(decode_manifest(&bytes).unwrap(), captured);
}

#[test]
fn every_bound_field_changes_the_generation_id() {
    let baseline = derive_generation_id(&encode_manifest(&sample()).unwrap());
    let mut mutations = artifact_mutations();
    mutations.extend(machine_mutations());
    assert_eq!(mutations.len(), 40);
    let mut seen = vec![baseline.clone()];
    for (name, mutate) in mutations {
        let mut manifest = sample();
        mutate(&mut manifest);
        let id = derive_generation_id(&encode_manifest(&manifest).unwrap());
        assert_ne!(id, baseline, "{name} did not change the GenerationId");
        assert!(!seen.contains(&id), "{name} collided with another mutation");
        seen.push(id);
    }
}

#[test]
fn identity_is_a_pure_function_of_manifest_bytes() {
    let first = encode_manifest(&sample()).unwrap();
    let second = encode_manifest(&sample()).unwrap();
    assert_eq!(first, second);
    assert_eq!(derive_generation_id(&first), derive_generation_id(&second));
}

#[test]
fn decoder_rejects_truncation_trailing_bytes_and_corruption() {
    let bytes = encode_manifest(&sample()).unwrap();
    for length in 0..bytes.len() {
        assert!(
            decode_manifest(&bytes[..length]).is_err(),
            "prefix {length} accepted"
        );
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(decode_manifest(&trailing).is_err());
    let mut magic = bytes.clone();
    magic[0] ^= 1;
    assert!(decode_manifest(&magic).is_err());
    let mut schema = bytes.clone();
    schema[9] = 2;
    assert_eq!(
        decode_manifest(&schema).unwrap_err().kind(),
        CompileErrorKind::Unsupported
    );
    let mut tag = bytes.clone();
    tag[12] = 3;
    assert!(decode_manifest(&tag).is_err());
    let media = ArtifactRole::ErofsRoot.media_type().as_bytes();
    let position = bytes.windows(media.len()).position(|w| w == media).unwrap();
    let mut wrong_media = bytes.clone();
    wrong_media[position] = b'X';
    assert!(decode_manifest(&wrong_media).is_err());
}

#[test]
fn decoder_rejects_duplicate_descriptors_and_unsupported_platforms() {
    let mut duplicate = sample();
    duplicate.overlay.templates[1].descriptor.digest = duplicate.root.descriptor.digest;
    let bytes = encode_manifest(&duplicate).unwrap();
    assert_eq!(
        decode_manifest(&bytes).unwrap_err().kind(),
        CompileErrorKind::InvalidInput
    );
    let mut unsorted = sample();
    unsorted.overlay.templates.swap(0, 1);
    assert!(encode_manifest(&unsorted).is_err());
    let mut arm = sample();
    arm.source.platform = OciPlatform::linux_arm64();
    assert!(encode_manifest(&arm).is_err());
    let mut nul = sample();
    nul.command_line.push(0);
    assert!(encode_manifest(&nul).is_err());
    let mut wrong_role = sample();
    wrong_role.kernel.descriptor.role = ArtifactRole::GuestAgent;
    assert!(encode_manifest(&wrong_role).is_err());
    let mut long = sample();
    long.guest_agent.build_provenance = "x".repeat(257);
    assert_eq!(
        encode_manifest(&long).unwrap_err().kind(),
        CompileErrorKind::LimitExceeded
    );
}
