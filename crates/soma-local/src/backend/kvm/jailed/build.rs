//! Opening everything a jailed machine needs, on the side that still has a filesystem.
//!
//! This is the half of the split that keeps the paths. It resolves the Generation store, opens
//! the hypervisor device and the immutable root, opens the two published snapshot objects,
//! clones this Instance's private head and unlinks it, and creates the pre-connected control
//! socket. What crosses into the jail is that set of descriptors and nothing else: no path, no
//! directory handle, no environment, and no name.

use std::fs::{File, OpenOptions};
use std::os::fd::OwnedFd;

use soma::{BackendFailureKind, InstanceId};
use soma_jail::{
    ArtifactKind, CgroupLimits, ControlSocket, CpuMax, DescriptorManifest, DescriptorRole,
    Identity, JailSpec, LeafName, Phase, Resources, Rlimits,
};

use super::anchors::Anchors;
use crate::backend::kvm::boot::private_head_from;
use crate::backend::kvm::{claim, prepared::PreparedGeneration};

/// Bytes in one mebibyte.
const MIB: u64 = 1024 * 1024;

/// The ephemeral identity every jailed worker runs as inside its own user namespace.
///
/// The value is fixed because it is never a host identity: each worker has its own user
/// namespace, so two workers holding the same number share nothing at all. It is only required
/// to be neither zero nor an overflow identity.
const WORKER_IDENTITY: Identity = Identity {
    uid: 60_001,
    gid: 60_001,
};

/// How much host memory one machine's jail may use beyond its guest RAM.
///
/// The worker maps the memory image privately, so the guest's own pages are charged to it, and
/// this covers the VMM's stacks, device buffers, and the retained operation receipts.
const HOST_OVERHEAD_BYTES: u64 = 512 * MIB;

/// Descriptors beyond the manifest and the standard streams that the child may hold.
///
/// The launcher seals the table, so this only has to cover the slots the profile itself needs
/// during setup rather than anything the machine opens later: it opens nothing.
const NOFILE_SLACK: u32 = 16;

/// Threads and processes one machine's jail may hold.
///
/// A machine runs its vCPU, its device loop, and its sandbox thread; the ceiling is generous
/// enough for the runtime's own helpers and finite enough that a worker cannot fork a host.
const PIDS_MAX: u32 = 32;

/// The descriptor roles one jailed machine receives, in manifest order.
pub(super) fn roles(overlay: bool) -> Vec<DescriptorRole> {
    let mut roles = vec![
        DescriptorRole::Kvm,
        DescriptorRole::RootDisk,
        DescriptorRole::Artifact(ArtifactKind::MemorySnapshot),
        DescriptorRole::Artifact(ArtifactKind::DeviceState),
    ];
    if overlay {
        roles.push(DescriptorRole::OverlayHead);
    }
    roles.push(DescriptorRole::Control);
    roles
}

/// What the broker opens for one machine.
pub(super) struct Opened {
    pub(super) resources: Resources,
    /// The supervisor's end of the control socket.
    pub(super) control: ControlSocket,
}

/// Opens every resource one jailed machine is built from.
pub(super) fn open(
    anchors: &Anchors,
    prepared: &PreparedGeneration,
    instance: &InstanceId,
    overlay: bool,
) -> Result<Opened, BackendFailureKind> {
    fn unavailable<E>(_: E) -> BackendFailureKind {
        BackendFailureKind::Unavailable
    }
    let kvm = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .map_err(unavailable)?;
    let root = prepared
        .open_artifact(&prepared.manifest.root.descriptor)
        .map_err(unavailable)?;
    let (memory_descriptor, overlay_descriptor, state_descriptor) =
        claim::snapshot(prepared).ok_or(BackendFailureKind::Unavailable)?;
    let memory = prepared
        .open_artifact(&memory_descriptor)
        .map_err(unavailable)?;
    let state = prepared
        .open_artifact(&state_descriptor)
        .map_err(unavailable)?;
    // The head is this Instance's private disk, cloned from the snapshot's own quiesced
    // template and unlinked before it is handed over, so nothing on the filesystem outlives
    // the machine that owns it and no name for it ever reaches the jail.
    let head = overlay
        .then(|| {
            let template = prepared
                .open_artifact(&overlay_descriptor)
                .map_err(unavailable)?;
            private_head_from(template, instance)
        })
        .transpose()?;
    // The worker's end is created here rather than by the worker, which is the whole point
    // of the split: a jailed process may not create, bind, or accept a socket, and this one
    // never has to.
    let (supervisor, worker) = ControlSocket::pair().map_err(unavailable)?;

    let mut descriptors = vec![
        (DescriptorRole::Kvm, OwnedFd::from(kvm)),
        (DescriptorRole::RootDisk, OwnedFd::from(root)),
        (
            DescriptorRole::Artifact(ArtifactKind::MemorySnapshot),
            OwnedFd::from(memory),
        ),
        (
            DescriptorRole::Artifact(ArtifactKind::DeviceState),
            OwnedFd::from(state),
        ),
    ];
    if let Some(head) = head {
        descriptors.push((DescriptorRole::OverlayHead, OwnedFd::from(head)));
    }
    descriptors.push((DescriptorRole::Control, worker));

    Ok(Opened {
        resources: Resources {
            null: OwnedFd::from(File::open("/dev/null").map_err(unavailable)?),
            log: OwnedFd::from(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(anchors.log_for(instance.as_str()))
                    .map_err(unavailable)?,
            ),
            executable: OwnedFd::from(File::open(&anchors.worker).map_err(unavailable)?),
            descriptors,
        },
        control: supervisor,
    })
}

/// The jail one machine of this shape runs in.
pub(super) fn spec(
    instance: &InstanceId,
    memory_mib: u64,
    disk_mib: u64,
    overlay: bool,
) -> Result<JailSpec, BackendFailureKind> {
    let roles = roles(overlay);
    let manifest =
        DescriptorManifest::new(roles).map_err(|_| BackendFailureKind::WorkloadRejected)?;
    let slots = u32::try_from(manifest.roles().len()).unwrap_or(u32::MAX);
    Ok(JailSpec {
        identity: WORKER_IDENTITY,
        leaf: LeafName::new(&format!("soma-{}", instance.as_str()))
            .map_err(|_| BackendFailureKind::WorkloadRejected)?,
        limits: CgroupLimits {
            memory_max_bytes: memory_mib
                .saturating_mul(MIB)
                .saturating_add(HOST_OVERHEAD_BYTES),
            // The machine contract fixes one vCPU, so one CPU is the whole machine's share.
            cpu_max: CpuMax {
                quota_us: 100_000,
                period_us: 100_000,
            },
            pids_max: PIDS_MAX,
            io_max: None,
        },
        rlimits: Rlimits {
            nofile: slots.saturating_add(NOFILE_SLACK),
            nproc: PIDS_MAX,
            // The private head is the only thing a machine writes, so its declared size is the
            // largest file this worker may ever produce.
            fsize_bytes: disk_mib.saturating_mul(MIB).max(MIB),
            address_space_bytes: None,
        },
        manifest,
        phase: Phase::Startup,
    })
}
