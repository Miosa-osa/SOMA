//! Contract, shape, snapshot, repair, and Template field validation.

use crate::generation::{
    artifacts::{ArtifactRole, Sha256Digest},
    contracts,
    manifest::{GenerationManifest, OverlayTemplate, SnapshotBinding, fixture},
};

use super::{Incompatibility, MIB, accepted, profile, rejected, require_profile};

#[test]
fn machine_artifact_versions_and_sizes_are_validated() {
    let profile = profile();
    rejected(Incompatibility::KernelContract, |m| {
        m.kernel.elf_pvh_contract_version = 2;
    });
    rejected(Incompatibility::KernelContract, |m| {
        m.kernel.cpu_architecture = "aarch64".to_owned();
    });
    rejected(Incompatibility::ZeroDigest, |m| {
        m.kernel.config_digest = Sha256Digest::from_bytes([0; 32]);
    });
    rejected(Incompatibility::KernelSize, |m| {
        m.kernel.descriptor.size = 0;
    });
    rejected(Incompatibility::KernelSize, |m| {
        m.kernel.descriptor.size = profile.max_kernel_bytes + 1;
    });
    rejected(Incompatibility::InitramfsLayout, |m| {
        m.initramfs.layout_version = 2;
    });
    rejected(Incompatibility::InitramfsSize, |m| {
        m.initramfs.descriptor.size = profile.max_initramfs_bytes + 1;
    });
    rejected(Incompatibility::GuestAgentSize, |m| {
        m.guest_agent.descriptor.size = profile.max_executable_bytes + 1;
    });
    rejected(Incompatibility::GuestAgentProvenance, |m| {
        m.guest_agent.build_provenance = String::new();
    });
    rejected(Incompatibility::GuestAgentProvenance, |m| {
        m.guest_agent.build_provenance = "p".repeat(257);
    });
    rejected(Incompatibility::GuestProtocol, |m| {
        m.guest_agent.application_protocol_version = 2;
    });
    rejected(Incompatibility::GuestProtocol, |m| {
        m.guest_agent.handshake_protocol_version = 2;
    });

    let mut manifest = fixture::profile_v1();
    manifest.kernel.descriptor.size = profile.max_kernel_bytes;
    manifest.guest_agent.build_provenance = "p".repeat(256);
    accepted(&manifest);
}

#[test]
fn every_bound_contract_statement_must_be_the_pinned_one() {
    rejected(Incompatibility::CommandLine, |m| {
        m.command_line = b"console=ttyS0".to_vec();
    });
    for mutate in [
        |m: &mut GenerationManifest| m.machine_contract.version = 2,
        |m: &mut GenerationManifest| {
            m.device_contract.digest = Sha256Digest::from_bytes([9; 32]);
        },
        |m: &mut GenerationManifest| m.cpu_template.version = 7,
    ] {
        rejected(Incompatibility::ContractStatement, mutate);
    }
}

#[test]
fn the_machine_shape_is_bounded_aligned_and_version_locked() {
    rejected(Incompatibility::MemorySize, |m| {
        m.shape.memory_bytes = 128 * MIB - 4096;
    });
    rejected(Incompatibility::MemorySize, |m| {
        m.shape.memory_bytes = 3 * 1024 * MIB + 4096;
    });
    rejected(Incompatibility::MemoryAlignment, |m| {
        m.shape.memory_bytes = 512 * MIB + 1;
    });
    rejected(Incompatibility::VcpuCount, |m| {
        m.shape.vcpu_count = 2;
    });
    rejected(Incompatibility::VcpuCount, |m| {
        m.shape.vcpu_count = 0;
    });
    rejected(Incompatibility::MemorySlotVersion, |m| {
        m.shape.memory_slot_layout_version = 2;
    });
    rejected(Incompatibility::LaunchPageVersion, |m| {
        m.shape.launch_page_layout_version = 2;
    });

    for edge in [128 * MIB, 3 * 1024 * MIB] {
        let mut manifest = fixture::profile_v1();
        manifest.shape.memory_bytes = edge;
        accepted(&manifest);
    }
}

#[test]
fn a_captured_snapshot_binding_is_bounded_and_version_locked() {
    let mut manifest = fixture::profile_v1();
    manifest.snapshot = fixture::captured_snapshot();
    accepted(&manifest);

    rejected(Incompatibility::SnapshotBinding, |m| {
        m.snapshot = SnapshotBinding::Captured {
            format_version: 2,
            memory: fixture::descriptor(ArtifactRole::MemorySnapshot, 0x20, 4096),
            state: fixture::descriptor(ArtifactRole::StateManifest, 0x21, 4096),
            capture_point_version: contracts::SNAPSHOT_CAPTURE_POINT_VERSION,
        };
    });
    rejected(Incompatibility::SnapshotBinding, |m| {
        m.snapshot = SnapshotBinding::Captured {
            format_version: contracts::SNAPSHOT_FORMAT_VERSION,
            memory: fixture::descriptor(ArtifactRole::MemorySnapshot, 0x20, 0),
            state: fixture::descriptor(ArtifactRole::StateManifest, 0x21, 4096),
            capture_point_version: contracts::SNAPSHOT_CAPTURE_POINT_VERSION,
        };
    });
}

#[test]
fn repair_and_template_fields_must_agree_with_the_rest_of_the_manifest() {
    rejected(Incompatibility::RepairPolicy, |m| {
        m.repair.policy_version = 2;
    });
    rejected(Incompatibility::RepairPolicy, |m| {
        m.repair.readiness_command_digest = Sha256Digest::from_bytes([3; 32]);
    });
    rejected(Incompatibility::WritableStorage, |m| {
        m.template.writable_storage_bytes = 512 * MIB;
    });
    rejected(Incompatibility::WritableStorage, |m| {
        // A class the profile certifies but this Generation never built a template for.
        m.overlay.templates.truncate(1);
        m.overlay.maximum_capacity = 256 * MIB;
        m.template.writable_storage_bytes = 1024 * MIB;
    });
    rejected(Incompatibility::Ttl, |m| {
        m.template.ttl_seconds = 0;
    });
    rejected(Incompatibility::Ttl, |m| {
        m.template.ttl_seconds = crate::generation::template::MAX_TTL_SECONDS + 1;
    });

    let mut manifest = fixture::profile_v1();
    manifest.template.ttl_seconds = crate::generation::template::MAX_TTL_SECONDS;
    accepted(&manifest);
}

#[test]
fn the_overlay_template_list_and_the_declared_class_are_cross_checked() {
    let mut manifest = fixture::profile_v1();
    manifest.overlay.templates.push(OverlayTemplate {
        capacity: 4096 * MIB,
        descriptor: fixture::descriptor(ArtifactRole::OverlayTemplate, 0x07, 4096 * MIB),
    });
    manifest.overlay.maximum_capacity = 4096 * MIB;
    let error = require_profile(&manifest, &profile()).expect_err("an uncertified class");
    assert_eq!(
        error.incompatibility(),
        Some(Incompatibility::OverlayCapacity)
    );
}
