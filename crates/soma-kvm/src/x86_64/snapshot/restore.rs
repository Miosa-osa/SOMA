//! Live restore of a captured snapshot into a fresh Instance.
//!
//! The order is the machine contract's twelve steps, driven through the codec's typed
//! schedule. Constant-size compatibility is decided before the memory object is mapped, the
//! memory object is mapped privately and never copied, every backend is fresh, eventfds and
//! interrupt routes exist before any captured interrupt state is armed, the fresh launch page
//! goes into its own slot which the snapshot never contained, and the vCPU resumes only after
//! every state constructor has succeeded.

mod devices;
mod guest_memory;
mod handle;
mod readiness;
mod sections;
mod sterile;

pub use handle::Restored;

use std::cell::Cell;

use vmm_sys_util::eventfd::EventFd;

use self::{
    devices::{Identity, recreate_devices, verify},
    sections::{Sections, net_mac, section, vsock_cid},
};
use super::{
    error::{Artifact, SnapshotError},
    marker,
    objects::SnapshotObjects,
    platform, profile, vcpu,
};
use crate::snapshot::{
    Digest, compatibility,
    manifest::Manifest,
    restore::{RestoreSequence, RestoreStep},
    section::SectionRole,
};
use crate::virtio::{DeviceSet, Slot};
use crate::x86_64::sandbox::NetworkAttachment;
use crate::x86_64::{
    Hypervisor, Machine,
    devices::SandboxDisks,
    error::{MachineError, Phase},
    events::{IrqLines, NotifyFds},
    launch_page::LaunchPageSlot,
    layout::GuestLayout,
    memory::{GuestRam, RamMapping},
    sandbox::{Milestone, SandboxMachine, Timeline, restored::RestoredParts},
    serial::SERIAL_GSI,
    timing::Stopwatch,
};
use sterile::SterileFacts;
pub use sterile::{Sterile, SterileRequest};

/// What a caller asks a restore to produce.
pub struct RestoreRequest {
    /// The published snapshot, as the handles this machine reads it through.
    pub objects: SnapshotObjects,
    /// Where this machine's `/dev/kvm` handle comes from.
    pub hypervisor: Hypervisor,
    /// The immutable root and, when the Generation declared writable storage, the
    /// Instance-private overlay head cloned from `overlay.raw`.
    ///
    /// A durable volume's entry here is the *placeholder* the device is declared against: its
    /// length is the capacity the guest is told, and it carries no tenant data.
    pub disks: SandboxDisks,
    /// The Instance's own durable volume file, opened by the caller from the path the host's
    /// volume store certified, or `None` for a Generation that declares no volume.
    ///
    /// It is bound over the placeholder before the guest runs, so a snapshot never carries
    /// tenant data and a machine that never received the file refuses to start rather than
    /// serve an empty disk.
    pub volume: Option<std::fs::File>,
    /// The optional devices this Generation declared; a snapshot of a different set is refused.
    pub devices: DeviceSet,
    /// The fresh vsock context identifier this Instance is assigned.
    pub guest_cid: u32,
    /// Guest RAM the caller expects, from the Generation shape rather than from the snapshot.
    pub memory_bytes: u64,
    /// The vCPU count the Generation declared, from its shape rather than from the snapshot.
    pub vcpus: u16,
    /// The machine contract the Generation was built under; a snapshot of another is refused.
    pub contract: crate::contract::MachineContract,
    /// Whether to re-hash the memory object and the overlay template before mapping.
    ///
    /// This is the installation and audit boundary, not the warm request path: it reads every
    /// byte of both objects.
    pub verify_artifacts: bool,
    /// The assigned network bundle, when this Instance was given one.
    pub network: Option<NetworkAttachment>,
    /// The hotplug region the Generation declared, or `None` for a machine without one; a
    /// snapshot that disagrees is refused. The region always restores empty, and its slot is registered with the rest.
    pub hotplug: Option<crate::memory_layout::HotplugRegion>,
}

/// What the restored machine is, taken from the manifest rather than from the caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoreFacts {
    /// The digest of the exact snapshot state object this Instance was restored from.
    pub snapshot: Digest,
    pub candidate_id: [u8; 32],
    pub memory_bytes: u64,
    /// The console line the agent printed at the capture point.
    pub repair_point_line: Vec<u8>,
    /// The placeholder MAC the snapshot carries; restore keeps it.
    pub mac: [u8; 6],
    /// The context identifier the captured machine held, replaced by the fresh assignment.
    pub captured_cid: u64,
    /// The fresh context identifier this Instance holds.
    pub guest_cid: u32,
    /// The number of vCPUs the restored machine actually runs, read from the manifest.
    pub vcpus: u16,
}

/// Restores one Instance from a published snapshot.
///
/// # Errors
///
/// Returns the first typed failure; every resource acquired before it is released in reverse
/// ownership order as the partially built machine unwinds.
pub fn restore(request: RestoreRequest) -> Result<Restored, SnapshotError> {
    let RestoreRequest {
        objects,
        hypervisor,
        disks,
        volume,
        devices,
        guest_cid,
        memory_bytes,
        vcpus,
        contract,
        verify_artifacts,
        network,
        hotplug,
    } = request;
    let SandboxDisks {
        root,
        overlay,
        volume: disks_placeholder,
    } = disks;
    let overlay_capacity_bytes = overlay
        .as_ref()
        .map(|overlay| {
            overlay
                .metadata()
                .map(|metadata| metadata.len())
                .map_err(|error| SnapshotError::io(Artifact::Overlay, "metadata", &error))
        })
        .transpose()?;
    // The volume device is declared with the capacity of whichever file the caller supplied:
    // the Instance's own volume, when the launch has one, or the placeholder whose length
    // declares the class. Either way the guest is told the class the Generation fixed, and the
    // file the launch opened is what it writes, so a snapshot never carries tenant data.
    let volume_capacity_bytes = volume
        .as_ref()
        .or(disks_placeholder.as_ref())
        .map(|file| {
            file.metadata()
                .map(|metadata| metadata.len())
                .map_err(|error| SnapshotError::io(Artifact::Overlay, "metadata", &error))
        })
        .transpose()?;
    restore_sterile(SterileRequest {
        objects,
        hypervisor,
        root,
        overlay_capacity_bytes,
        volume_capacity_bytes,
        devices,
        memory_bytes,
        vcpus,
        contract,
        verify_artifacts,
        hotplug,
    })?
    // The Instance's own volume file arrives with the head: the placeholder declared the device
    // and this is what the guest writes, bound over the placeholder before it runs.
    .assign(overlay, volume, guest_cid, network)
}

mod sterile_entry;
pub use sterile_entry::restore_sterile;
