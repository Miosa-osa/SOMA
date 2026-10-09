//! Turning one prepared Generation into everything a machine needs to boot.

use soma::{BackendFailureKind, InstanceId};
use soma_generation::generation_manifest::SnapshotBinding;
use soma_guest::{LaunchNetwork, SecretFile};
use soma_kvm::MachineContract;
use soma_kvm::x86_64::{Hypervisor, SandboxDisks, SnapshotObjects};

use super::identity::{LaunchIdentity, generation_bytes, now_unix_nanos};
use super::prepared::PreparedGeneration;
use soma_vmm::sandbox::{Boot, ColdBootInputs, Network, Source, cold_boot_config};

mod head;

pub(in crate::backend::kvm) use head::private_head_from;

/// Opens the prepared artifacts and gives this Instance its own writable overlay head.
///
/// The overlay template in the store is sterile and shared, so it is never opened writable:
/// each Instance receives a private copy, and two Instances of one Generation therefore
/// cannot observe each other's writes.
pub(super) fn boot_for(
    prepared: &PreparedGeneration,
    memory_mib: u64,
    identity: LaunchIdentity,
    network: Network,
    secrets: Vec<SecretFile>,
) -> Result<Boot, BackendFailureKind> {
    let instance = &InstanceId::new(hex(identity.instance))
        .map_err(|_| BackendFailureKind::WorkloadRejected)?;
    let manifest = &prepared.manifest;
    let open = |descriptor| {
        prepared
            .open_artifact(descriptor)
            .map_err(|_| BackendFailureKind::Unavailable)
    };
    let kernel = open(&manifest.kernel.descriptor)?;
    let initramfs = open(&manifest.initramfs.descriptor)?;
    let root = open(&manifest.root.descriptor)?;
    // A Generation that declared no writable storage published no sterile template, and the
    // whole point of it is that no head is cloned on the request path: the clone of a private
    // head is the largest and most variable cost between admission and a launched machine.
    let devices = manifest.device_set();
    // The shape and the contract are the Generation's own statements, so a launch serves the
    // machine the Generation certified rather than the shape the request happened to carry.
    let vcpus = manifest.shape.vcpu_count;
    let contract = MachineContract::require(manifest.machine_contract.version)
        .map_err(|_| BackendFailureKind::WorkloadRejected)?;
    let template = if devices.overlay() {
        Some(
            manifest
                .overlay
                .templates
                .first()
                .ok_or(BackendFailureKind::Unavailable)?,
        )
    } else {
        None
    };
    // A prepared entry may carry a snapshot taken once for the whole Generation. When it does,
    // this launch resumes that machine instead of booting a kernel, which is the difference
    // between hundreds of milliseconds and tens on the request path.
    let guest_cid = identity.guest_cid;
    let source = if let SnapshotBinding::Captured {
        memory,
        overlay: snapshot_overlay,
        state,
        ..
    } = manifest.snapshot
    {
        {
            // The restore clones its own head from the snapshot's sterile overlay template, not
            // from the Candidate's, because the captured machine has already written to it.
            let overlay = devices
                .overlay()
                .then(|| open(&snapshot_overlay).and_then(|file| private_head_from(file, instance)))
                .transpose()?;
            let objects = SnapshotObjects::adopt(open(&state)?, open(&memory)?, None);
            Source::Restore {
                objects,
                hypervisor: Hypervisor::Device,
                disks: SandboxDisks { root, overlay },
                devices,
                memory_bytes: memory_mib * MIB,
                vcpus,
                contract,
            }
        }
    } else {
        {
            let overlay = template
                .map(|template| {
                    open(&template.descriptor).and_then(|file| private_head_from(file, instance))
                })
                .transpose()?;
            Source::ColdBoot(cold_boot_config(ColdBootInputs {
                kernel,
                initramfs,
                root,
                overlay,
                ram_bytes: memory_mib * MIB,
                guest_cid,
                devices,
                vcpus,
                contract,
            }))
        }
    };
    Ok(Boot {
        source,
        generation: generation_bytes(&prepared.id)?,
        instance: identity.instance,
        operation: identity.operation,
        guest_cid,
        network,
        secrets,
    })
}

/// The Instance identity as the lowercase hexadecimal its portable form is written in.
fn hex(instance: [u8; 16]) -> String {
    use std::fmt::Write as _;
    instance
        .iter()
        .fold(String::with_capacity(32), |mut out, byte| {
            let _ignored = write!(out, "{byte:02x}");
            out
        })
}

/// Bytes in one mebibyte.
const MIB: u64 = 1024 * 1024;

/// The link-down placeholder network every guest is given today.
///
/// The addresses are fixed because nothing routes them: the device exists so the guest's repair
/// step has one to configure, and no packet leaves the machine.
///
/// The context identifier is not fixed. The guest agent checks the identifier its own vsock
/// device reports against the one the launch page names, and refuses the session when they
/// disagree, which is what binds the transport the session runs over to this Instance's
/// authority. So this must be given the same identifier the machine was built with rather than
/// a constant: a launch page naming a different one leaves a correctly built machine unable to
/// form a session at all.
pub(super) fn link_down_network(guest_cid: u32) -> Result<LaunchNetwork, BackendFailureKind> {
    LaunchNetwork::new(
        guest_cid,
        1,
        [0x02, 0x53, 0x4f, 0x4d, 0x41, 0x01],
        [10, 0, 0, 2],
        24,
        [10, 0, 0, 1],
        [10, 0, 0, 1],
        now_unix_nanos(),
    )
    .map_err(|_| BackendFailureKind::Unavailable)
}

#[cfg(test)]
#[path = "boot_tests.rs"]
mod tests;
