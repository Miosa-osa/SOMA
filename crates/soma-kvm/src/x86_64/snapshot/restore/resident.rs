//! Verify and rebuild a resident checkpoint for restore or fork.

use std::{fs::File, os::unix::fs::MetadataExt as _};

use super::{
    Restored, SnapshotError, Sterile, Timeline,
    devices::Declared,
    sterile_entry::{facts, open_kvm, rebuild, validate_state},
};
use crate::{
    contract::MachineContract,
    memory_layout::HotplugRegion,
    snapshot::{
        ArtifactAttestation, Digest, RestoreAttestations, device_state::DeviceSpecific,
        manifest::CheckpointManifest,
    },
    virtio::{DeviceSet, Slot},
    x86_64::{Hypervisor, LAUNCH_PAGE_SIZE, SnapshotObjects, snapshot::Artifact},
};

/// The broker-owned inputs for reconstructing one stopped Instance.
pub struct ResidentRestoreRequest {
    /// An authorized state manifest, memory object and optional saved overlay.
    pub objects: SnapshotObjects,
    pub hypervisor: Hypervisor,
    /// Read-only immutable Generation root.
    pub root: File,
    /// A private writable copy of the saved overlay on a distinct inode.
    pub head: Option<File>,
    /// A private, locked writable copy of the saved durable volume on a distinct inode.
    pub volume: Option<File>,
    pub instance_id: [u8; 16],
    pub candidate_id: [u8; 32],
    pub state_digest: Digest,
    pub devices: DeviceSet,
    pub guest_cid: u32,
    pub memory_bytes: u64,
    pub vcpus: u16,
    pub contract: MachineContract,
    pub hotplug: Option<HotplugRegion>,
    /// The broker's records of the checkpoint memory, the Generation root, and a head it has just
    /// cloned from an attested saved overlay, filled only by the broker that opened the
    /// descriptors. A file whose stamp, size, or digest differs is hashed; a durable volume
    /// always is.
    pub attestations: RestoreAttestations,
}

/// An unstarted Instance with a disconnected control channel.
pub struct PreparedCheckpoint {
    restored: Restored,
    instance_id: [u8; 16],
    fork: bool,
}

impl PreparedCheckpoint {
    /// Attaches a freshly leased network while the restored machine is stopped.
    /// The link remains closed until the authenticated session enables it.
    pub fn attach_network(&self, network: crate::x86_64::NetworkAttachment) {
        self.restored.machine.attach_network(network);
    }

    /// Starts the captured processors with fresh reconnect authority and the network link down.
    ///
    /// The returned VM must still prove its new authenticated session.
    ///
    /// # Errors
    ///
    /// Refuses launch material that names a different Instance and consumes the prepared VM
    /// if resume fails.
    pub fn resume(mut self, page: &[u8; LAUNCH_PAGE_SIZE]) -> Result<Restored, SnapshotError> {
        // The standard and fork pages share the identity offsets but use distinct schemas.
        const SCHEMA_OFFSET: std::ops::Range<usize> = 16..18;
        const RESTORE_SCHEMA: [u8; 2] = 3_u16.to_be_bytes();
        const FORK_SCHEMA: [u8; 2] = 4_u16.to_be_bytes();
        let session = crate::snapshot::readiness::page_session(page)
            .ok_or(crate::snapshot::readiness::ReadinessRefusal::Unbound)?;
        let schema = if self.fork {
            FORK_SCHEMA
        } else {
            RESTORE_SCHEMA
        };
        if page[SCHEMA_OFFSET] != schema || session.0 != self.instance_id {
            return Err(SnapshotError::NotQuiescent(
                "reconnect authority names a different Instance",
            ));
        }
        self.restored.resume(page)?;
        Ok(self.restored)
    }
}

/// Rebuilds a broker-authorized checkpoint for the original Instance after its owner stopped.
///
/// All mapped artifacts must remain private to the trusted broker and immutable for the lifetime
/// of the restored machine. The memory file and the root are hashed unless the broker's
/// attestation vouches for their exact current stamps, and so is a head the broker has just
/// cloned from an attested saved overlay; the durable volume always is.
/// The broker owns the same-Instance lease and must exclude another active owner. This entry
/// point performs no scheduling or tenant authorization of its own.
///
/// # Errors
///
/// Refuses an identity, compatibility, or artifact mismatch before VM creation, or returns the
/// first typed machine setup error.
pub fn prepare_checkpoint(
    request: ResidentRestoreRequest,
) -> Result<PreparedCheckpoint, SnapshotError> {
    let instance_id = request.instance_id;
    prepare_for(request, instance_id)
}

/// Forks a resident checkpoint into a newly allocated destination Instance.
///
/// The trusted broker authorizes access to the source and supplies a separate writable head for
/// every destination. Each fork uses a fresh destination CID and independently authenticates.
///
/// # Errors
/// Refuses a missing or reused Instance identity or any incompatible checkpoint artifact.
pub fn prepare_fork(
    request: ResidentRestoreRequest,
    destination: [u8; 16],
) -> Result<PreparedCheckpoint, SnapshotError> {
    if destination == [0; 16] || destination == request.instance_id {
        return Err(SnapshotError::NotQuiescent(
            "a checkpoint fork requires a different nonzero Instance",
        ));
    }
    prepare_for(request, destination)
}

fn prepare_for(
    request: ResidentRestoreRequest,
    destination: [u8; 16],
) -> Result<PreparedCheckpoint, SnapshotError> {
    let ResidentRestoreRequest {
        mut objects,
        hypervisor,
        mut root,
        mut head,
        mut volume,
        instance_id,
        candidate_id,
        state_digest,
        devices,
        guest_cid,
        memory_bytes,
        vcpus,
        contract,
        hotplug,
        attestations,
    } = request;
    let timeline = Timeline::new();
    if devices.volume() != volume.is_some() {
        return Err(SnapshotError::NotQuiescent(
            "resident restore volume does not match the declared devices",
        ));
    }
    let state_bytes = objects.state_bytes()?;
    if Digest::of(&state_bytes) != state_digest {
        return Err(SnapshotError::StagingDigestMismatch(Artifact::State));
    }
    let manifest = CheckpointManifest::decode(&state_bytes)?;
    if manifest.instance_id() != &instance_id
        || manifest.header().candidate_id.as_bytes() != &candidate_id
    {
        return Err(SnapshotError::NotQuiescent(
            "checkpoint identity differs from the authorized Instance and Generation",
        ));
    }
    let declared = Declared {
        overlay_capacity_bytes: head
            .as_ref()
            .map(|file| metadata(Artifact::Overlay, file).map(|info| info.len()))
            .transpose()?,
        volume_capacity_bytes: volume
            .as_ref()
            .map(|file| metadata(Artifact::Volume, file).map(|info| info.len()))
            .transpose()?,
        set: devices,
        contract,
    };
    let kvm = open_kvm(hypervisor)?;
    let state = validate_state(
        &kvm,
        manifest.state(),
        memory_bytes,
        vcpus,
        declared,
        hotplug,
    )?;
    super::attested::verify_memory(
        objects.memory_handle(),
        &manifest.header().memory,
        attestations.memory,
    )?;
    let root_record = (Slot::Root, Artifact::Root, attestations.root);
    verify_disk(root_record, &mut root, &state)?;
    verify_head(
        head.as_mut(),
        objects.overlay_template(),
        &state,
        attestations.head,
    )?;
    if let Some(file) = &mut volume {
        verify_disk((Slot::Volume, Artifact::Volume, None), file, &state)?;
    }
    let facts = facts(manifest.state(), state_digest, Vec::new(), &state)?;
    let (machine, sequence) = rebuild(
        kvm,
        root,
        objects.memory(),
        &state,
        &facts,
        declared,
        hotplug,
        timeline,
    )?;
    let restored = Sterile {
        machine,
        facts,
        sequence,
    }
    .assign(head, volume, guest_cid, None, None)?;
    Ok(PreparedCheckpoint {
        restored,
        instance_id: destination,
        fork: destination != instance_id,
    })
}

fn verify_head(
    head: Option<&mut File>,
    saved: Option<&mut File>,
    state: &super::Sections,
    attestation: Option<ArtifactAttestation>,
) -> Result<(), SnapshotError> {
    match (head, saved, state.slot(Slot::Overlay)) {
        (None, None, None) => Ok(()),
        (Some(head), Some(saved), Some(_)) => {
            let current = metadata(Artifact::Overlay, head)?;
            let original = metadata(Artifact::Overlay, saved)?;
            if (current.dev(), current.ino()) == (original.dev(), original.ino()) {
                return Err(SnapshotError::NotQuiescent(
                    "the writable head aliases the saved overlay",
                ));
            }
            verify_disk((Slot::Overlay, Artifact::Overlay, attestation), head, state)
        }
        _ => Err(SnapshotError::DeviceStateNotCanonical(Slot::Overlay)),
    }
}

/// Checks one disk against its captured block record. The durable volume passes `None` and is
/// always hashed; the root and a freshly cloned head may carry the broker's attestation.
fn verify_disk(
    (slot, artifact, attestation): (Slot, Artifact, Option<ArtifactAttestation>),
    file: &mut File,
    state: &super::Sections,
) -> Result<(), SnapshotError> {
    let Some(DeviceSpecific::Block(block)) = state
        .slot(slot)
        .map(crate::snapshot::device_state::DeviceState::specific)
    else {
        return Err(SnapshotError::DeviceStateNotCanonical(slot));
    };
    let info = metadata(artifact, file)?;
    if block.capacity_sectors.checked_mul(512) != Some(info.len()) {
        return Err(SnapshotError::DeviceStateNotCanonical(slot));
    }
    let digest = super::attested::digest(artifact, file, &info, attestation, block.image_digest)?;
    if digest != block.image_digest {
        return Err(SnapshotError::StagingDigestMismatch(artifact));
    }
    Ok(())
}

fn metadata(artifact: Artifact, file: &File) -> Result<std::fs::Metadata, SnapshotError> {
    let info = file
        .metadata()
        .map_err(|error| SnapshotError::io(artifact, "stat", &error))?;
    if !info.is_file() {
        return Err(SnapshotError::NotQuiescent(
            "checkpoint restore requires regular files",
        ));
    }
    Ok(info)
}
