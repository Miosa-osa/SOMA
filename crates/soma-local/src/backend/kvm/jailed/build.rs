//! Opening everything a jailed machine needs, on the side that still has a filesystem.
//!
//! This is the half of the split that keeps the paths. It resolves the Generation store, opens
//! the hypervisor device and the immutable root, opens the two published snapshot objects,
//! clones this Instance's private head and unlinks it, and creates the pre-connected control
//! socket. What crosses into the jail is that set of descriptors and nothing else: no path, no
//! directory handle, no environment, and no name.

use std::fs::{File, OpenOptions};
use std::os::fd::OwnedFd;

use soma::BackendFailureKind;
use soma_jail::{
    ArtifactKind, CgroupLimits, ControlSocket, CpuMax, DescriptorManifest, DescriptorRole,
    Identity, JailSpec, LeafName, Phase, Resources, Rlimits,
};

use super::{Launching, anchors::Anchors};
use crate::backend::kvm::boot::private_head_from;
use crate::backend::kvm::{checkpoint::RestoreArtifacts, claim};

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

/// Descriptors shared by one machine, including the standard streams and launcher slots,
/// VM, interrupt and queue eventfds, and the device loop's epoll and wakeup descriptors.
const MACHINE_FD_BUDGET: u32 = 32;

/// Each vCPU holds a KVM descriptor and a duplicate of each of up to ten queue eventfds.
const VCPU_FD_BUDGET: u32 = 12;

/// Threads and processes one machine's jail may hold.
///
/// A machine runs its vCPU, its device loop, and its sandbox thread; the ceiling is generous
/// enough for the runtime's own helpers and finite enough that a worker cannot fork a host.
const PIDS_MAX: u32 = 32;
/// The scheduling period a CPU quota is a multiple of.
const CPU_PERIOD_US: u64 = 100_000;

/// The descriptor roles one jailed machine receives, in manifest order.
fn roles(overlay: bool, volume: bool, network: bool) -> Vec<DescriptorRole> {
    let mut roles = vec![
        DescriptorRole::Kvm,
        DescriptorRole::RootDisk,
        DescriptorRole::Artifact(ArtifactKind::MemorySnapshot),
        DescriptorRole::Artifact(ArtifactKind::DeviceState),
    ];
    if overlay {
        roles.push(DescriptorRole::OverlayHead);
    }
    if volume {
        roles.push(DescriptorRole::DataVolume);
    }
    if network {
        roles.extend([DescriptorRole::Tap, DescriptorRole::NetworkActivation]);
    }
    roles.push(DescriptorRole::Control);
    roles
}

/// What the broker opens for one machine.
pub(super) struct Opened {
    pub(super) resources: Resources,
    /// The supervisor's end of the control socket.
    pub(super) control: ControlSocket,
    pub(super) activation: Option<ControlSocket>,
    pub(super) recovery: Option<Recovery>,
}

pub(super) struct Recovery {
    pub(super) digest: soma_kvm::snapshot::Digest,
    pub(super) overlay: Option<File>,
    /// The memory record from the checkpoint and the root record from Generation admission.
    pub(super) attestations: soma_kvm::snapshot::RestoreAttestations,
}

impl Recovery {
    /// A private head for this Instance cloned from the checkpoint's saved overlay.
    fn clone_head(
        checkpoint: &RestoreArtifacts,
        instance: &soma::InstanceId,
    ) -> Result<Option<File>, BackendFailureKind> {
        checkpoint
            .overlay
            .as_ref()
            .map(|template| {
                let template = template
                    .try_clone()
                    .map_err(|_| BackendFailureKind::Unavailable)?;
                private_head_from(template, instance)
            })
            .transpose()
    }

    /// The broker's attestations of the checkpoint memory, of the Generation root as admission
    /// left it, and of the head this broker step has just cloned from the saved overlay.
    ///
    /// The head is vouched for here, after the clone and before any descriptor reaches the
    /// worker, and only when the saved overlay still has its recorded stamp.
    fn attestations(
        checkpoint: &RestoreArtifacts,
        head: Option<&File>,
        prepared: &crate::backend::kvm::prepared::PreparedGeneration,
    ) -> soma_kvm::snapshot::RestoreAttestations {
        let root = prepared.attestation(&prepared.manifest.root.descriptor);
        let head = checkpoint
            .overlay
            .as_ref()
            .zip(head)
            .and_then(|(saved, head)| {
                crate::backend::kvm::checkpoint::vouch_head(
                    saved,
                    checkpoint.overlay_attestation,
                    head,
                )
            });
        let memory = checkpoint.memory_attestation;
        soma_kvm::snapshot::RestoreAttestations { memory, root, head }
    }
}

/// Opens every resource one jailed machine is built from.
pub(super) fn open(
    anchors: &Anchors,
    launching: &Launching<'_>,
    checkpoint: Option<RestoreArtifacts>,
    volume: Option<File>,
    tap: Option<File>,
) -> Result<Opened, BackendFailureKind> {
    fn unavailable<E>(_: E) -> BackendFailureKind {
        BackendFailureKind::Unavailable
    }
    let prepared = launching.prepared;
    let instance = launching.instance;
    let overlay = prepared.manifest.device_set().overlay();
    let kvm = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .map_err(unavailable)?;
    let root = prepared
        .open_artifact(&prepared.manifest.root.descriptor)
        .map_err(unavailable)?;
    let (memory, state, head, recovery) = if let Some(checkpoint) = checkpoint {
        if checkpoint.overlay.is_some() != overlay {
            return Err(BackendFailureKind::WorkloadRejected);
        }
        let head = Recovery::clone_head(&checkpoint, instance)?;
        let attestations = Recovery::attestations(&checkpoint, head.as_ref(), prepared);
        (
            checkpoint.memory,
            checkpoint.state,
            head,
            Some(Recovery {
                digest: checkpoint.digest,
                overlay: checkpoint.overlay,
                attestations,
            }),
        )
    } else {
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
        (memory, state, head, None)
    };
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
    if let Some(volume) = volume {
        descriptors.push((DescriptorRole::DataVolume, OwnedFd::from(volume)));
    }
    let activation = if let Some(tap) = tap {
        let (broker, worker) = ControlSocket::pair().map_err(unavailable)?;
        descriptors.push((DescriptorRole::Tap, OwnedFd::from(tap)));
        descriptors.push((DescriptorRole::NetworkActivation, worker));
        Some(broker)
    } else {
        None
    };
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
        activation,
        recovery,
    })
}

/// The jail one machine of this shape runs in.
pub(super) fn spec(
    launching: &Launching<'_>,
    volume_bytes: Option<std::num::NonZeroU64>,
    network: bool,
) -> Result<JailSpec, BackendFailureKind> {
    let prepared = &launching.prepared.manifest;
    let (instance, memory_mib, disk_mib) =
        (launching.instance, launching.memory_mib, launching.disk_mib);
    let vcpus = prepared.shape.vcpu_count;
    let overlay = prepared.device_set().overlay();
    let roles = roles(overlay, volume_bytes.is_some(), network);
    let manifest =
        DescriptorManifest::new(roles).map_err(|_| BackendFailureKind::WorkloadRejected)?;
    let slots =
        u32::try_from(manifest.roles().len()).map_err(|_| BackendFailureKind::WorkloadRejected)?;
    Ok(JailSpec {
        identity: WORKER_IDENTITY,
        leaf: LeafName::new(&format!("soma-{}", instance.as_str()))
            .map_err(|_| BackendFailureKind::WorkloadRejected)?,
        limits: CgroupLimits {
            memory_max_bytes: memory_mib
                .saturating_mul(MIB)
                .saturating_add(HOST_OVERHEAD_BYTES),
            // The machine's share is one CPU per vCPU it was built with. A quota fixed at one CPU
            // would throttle an eight vCPU machine to an eighth of itself until its first resize.
            cpu_max: CpuMax {
                quota_us: u64::from(vcpus.max(1)) * CPU_PERIOD_US,
                period_us: CPU_PERIOD_US,
            },
            pids_max: PIDS_MAX,
            io_max: None,
        },
        rlimits: Rlimits {
            nofile: slots + MACHINE_FD_BUDGET + u32::from(vcpus.max(1)) * VCPU_FD_BUDGET,
            nproc: PIDS_MAX,
            // The broker may also pass a writable checkpoint descriptor for the guest RAM.
            // The head, volume, and captured guest RAM can each be a different size.
            fsize_bytes: memory_mib
                .max(disk_mib)
                .max(1)
                .saturating_mul(MIB)
                .max(volume_bytes.map_or(0, std::num::NonZeroU64::get)),
            address_space_bytes: None,
        },
        manifest,
        phase: Phase::Startup,
    })
}
