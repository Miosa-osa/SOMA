mod support;

use soma_generation::{
    ArtifactRole, derive_generation_id,
    generation_manifest::{GenerationManifest, SnapshotBinding, decode_manifest, encode_manifest},
    template::NetworkPolicyClass,
};
use support::manifest::{descriptor, digest, sample};

const GOLDEN_HEX: &str = include_str!("fixtures/somagen_v1.hex");
const GOLDEN_ID: &str = "sha256:4736e824d404433872a91908649cb8dbd0ac60140bb5aa8d55bdeb44edb225be";

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
        ("writable storage", |m| {
            m.template.writable_storage_bytes = 1 << 27;
        }),
        ("network class", |m| {
            m.template.network_policy_class = NetworkPolicyClass::RuntimeDefault;
        }),
        ("network digest", |m| {
            m.template.network_policy_digest = digest(0xb2);
        }),
        ("workload probe", |m| {
            m.template.workload_probe = Some(b"/usr/bin/true".to_vec());
        }),
        ("ttl", |m| m.template.ttl_seconds += 1),
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
    assert_eq!(mutations.len(), 45);
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
