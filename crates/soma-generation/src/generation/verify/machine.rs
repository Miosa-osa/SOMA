//! Host-profile compatibility of one decoded manifest: contracts, shape, and Template groups.
//!
//! These are the security-critical fields the Host acts on when it builds a machine, so every
//! one is bounded, version-checked, and cross-checked against the fields it must agree with.

use soma::NetworkPolicy;

use crate::generation::{
    contracts::{
        self, LAUNCH_PAGE_LAYOUT_VERSION, MEMORY_SLOT_LAYOUT_VERSION, REPAIR_POLICY_VERSION,
        SNAPSHOT_CAPTURE_POINT_VERSION, SNAPSHOT_FORMAT_VERSION,
    },
    error::CompileError,
    manifest::{GenerationManifest, SnapshotBinding},
    request::CompilerProfile,
    template::{
        MAX_TTL_SECONDS, MAX_WORKLOAD_PROBE_BYTES, NetworkPolicyClass, network_policy_digest,
    },
};

use super::{
    Incompatibility,
    profile::{descriptor_nonzero, require},
};

const MIB: u64 = 1024 * 1024;
/// The machine contract's guest RAM range and page granularity.
const MINIMUM_MEMORY_BYTES: u64 = 128 * MIB;
const MAXIMUM_MEMORY_BYTES: u64 = 3 * 1024 * MIB;
const MEMORY_PAGE_BYTES: u64 = 4096;

/// Rejects contract, shape, snapshot, repair, and Template fields that this host cannot honor.
///
/// # Errors
///
/// Returns one typed redacted rejection for the first violated invariant.
pub(super) fn require_machine(
    manifest: &GenerationManifest,
    profile: &CompilerProfile,
) -> Result<(), CompileError> {
    require(
        manifest.command_line == contracts::kernel_command_line_v1(),
        Incompatibility::CommandLine,
    )?;
    require(
        manifest.machine_contract == contracts::machine_contract_v1()
            && manifest.device_contract == contracts::device_contract_v1()
            && manifest.cpu_template == contracts::cpu_template_v1(),
        Incompatibility::ContractStatement,
    )?;
    require_shape(manifest)?;
    require_snapshot(manifest)?;
    require(
        manifest.repair.policy_version == REPAIR_POLICY_VERSION
            && manifest.repair.readiness_command_digest == contracts::readiness_command_digest(),
        Incompatibility::RepairPolicy,
    )?;
    require_template(manifest, profile)
}

fn require_shape(manifest: &GenerationManifest) -> Result<(), CompileError> {
    let shape = manifest.shape;
    require(
        (MINIMUM_MEMORY_BYTES..=MAXIMUM_MEMORY_BYTES).contains(&shape.memory_bytes),
        Incompatibility::MemorySize,
    )?;
    require(
        shape.memory_bytes.is_multiple_of(MEMORY_PAGE_BYTES),
        Incompatibility::MemoryAlignment,
    )?;
    require(shape.vcpu_count == 1, Incompatibility::VcpuCount)?;
    require(
        shape.memory_slot_layout_version == MEMORY_SLOT_LAYOUT_VERSION,
        Incompatibility::MemorySlotVersion,
    )?;
    require(
        shape.launch_page_layout_version == LAUNCH_PAGE_LAYOUT_VERSION,
        Incompatibility::LaunchPageVersion,
    )
}

fn require_snapshot(manifest: &GenerationManifest) -> Result<(), CompileError> {
    let SnapshotBinding::Captured {
        format_version,
        memory,
        overlay,
        state,
        capture_point_version,
    } = manifest.snapshot
    else {
        return Ok(());
    };
    require(
        format_version == SNAPSHOT_FORMAT_VERSION
            && capture_point_version == SNAPSHOT_CAPTURE_POINT_VERSION,
        Incompatibility::SnapshotBinding,
    )?;
    descriptor_nonzero(&memory, Incompatibility::SnapshotBinding)?;
    descriptor_nonzero(&overlay, Incompatibility::SnapshotBinding)?;
    descriptor_nonzero(&state, Incompatibility::SnapshotBinding)
}

fn require_template(
    manifest: &GenerationManifest,
    profile: &CompilerProfile,
) -> Result<(), CompileError> {
    let template = &manifest.template;
    require(
        profile
            .overlay_capacities
            .contains(&template.writable_storage_bytes)
            && manifest
                .overlay
                .templates
                .iter()
                .any(|entry| entry.capacity == template.writable_storage_bytes),
        Incompatibility::WritableStorage,
    )?;
    require_network(manifest)?;
    require_probe(manifest)?;
    require(
        template.ttl_seconds > 0 && template.ttl_seconds <= MAX_TTL_SECONDS,
        Incompatibility::Ttl,
    )
}

/// Requires the declared class and the canonical policy digest to name the same policy.
fn require_network(manifest: &GenerationManifest) -> Result<(), CompileError> {
    let declared = manifest.template.network_policy_digest;
    let isolated = network_policy_digest(&NetworkPolicy::isolated())?;
    let runtime = network_policy_digest(&NetworkPolicy::runtime_default())?;
    let agrees = match manifest.template.network_policy_class {
        NetworkPolicyClass::Isolated => declared == isolated,
        NetworkPolicyClass::RuntimeDefault => declared == runtime,
        NetworkPolicyClass::Explicit => declared != isolated && declared != runtime,
    };
    require(agrees, Incompatibility::NetworkPolicy)
}

/// Requires an explicit workload probe to name one absolute executable without control bytes.
fn require_probe(manifest: &GenerationManifest) -> Result<(), CompileError> {
    let Some(probe) = manifest.template.workload_probe.as_deref() else {
        return Ok(());
    };
    require(
        !probe.is_empty()
            && probe.len() <= MAX_WORKLOAD_PROBE_BYTES
            && probe.starts_with(b"/")
            && !probe.iter().any(u8::is_ascii_control),
        Incompatibility::WorkloadProbe,
    )
}
