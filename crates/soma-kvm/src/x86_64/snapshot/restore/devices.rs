//! Rebuilding the five devices with fresh backends and the certified transport state, and
//! verifying the two large objects at the installation boundary.
//!
//! Every backend is new: a fresh read-only handle on the immutable root, a fresh private
//! overlay head cloned from the snapshot's sterile template, a fresh network backend with the
//! link down, a fresh vsock endpoint with no connection, and a fresh entropy source. Only the
//! transport and queue state comes from the snapshot.

use std::path::Path;

use super::super::{
    artifacts::{self, SnapshotPaths},
    device,
    error::{Artifact, SnapshotError},
};
use super::sections::Sections;
use crate::snapshot::{device_state::DeviceSpecific, manifest::Manifest};
use crate::virtio::{MmioBus, Slot, SlotSnapshot};
use crate::x86_64::{
    Machine,
    devices::{self as machine_devices, DeviceIdentity, SandboxDisks},
};

/// The identity a restore installs: the snapshot's placeholder MAC, the context identifier it
/// was captured with, and the fresh one this Instance is assigned.
pub(super) struct Identity {
    pub(super) mac: [u8; 6],
    pub(super) captured_cid: u64,
    pub(super) guest_cid: u32,
}

pub(super) fn recreate_devices(
    machine: &Machine,
    disks: SandboxDisks,
    state: &Sections,
    identity: &Identity,
) -> Result<MmioBus, SnapshotError> {
    let captured_cid = u32::try_from(identity.captured_cid)
        .map_err(|_| SnapshotError::DeviceStateNotCanonical(Slot::Vsock))?;
    let devices = machine_devices::build_devices(
        disks,
        DeviceIdentity {
            guest_cid: captured_cid,
            guest_mac: identity.mac,
        },
    )?;
    let records = state
        .devices
        .iter()
        .zip(Slot::ALL)
        .map(|(certified, slot)| {
            Ok(SlotSnapshot {
                slot,
                transport: device::transport(certified),
                device: device::fresh_record(slot, &devices, certified)?,
            })
        })
        .collect::<Result<Vec<_>, SnapshotError>>()?;
    let records: [SlotSnapshot; 5] = records
        .try_into()
        .map_err(|_| SnapshotError::DeviceStateNotCanonical(Slot::Root))?;
    let mut bus = MmioBus::restore(devices, &records, &machine.shared_ram())?;
    // The fresh context identifier replaces the captured one; the transport-reset event the
    // vsock restore queued makes the guest driver re-read it before the agent connects.
    bus.vsock_mut()
        .device_mut()
        .set_guest_cid(u64::from(identity.guest_cid))
        .map_err(|_| SnapshotError::DeviceStateNotCanonical(Slot::Vsock))?;
    Ok(bus)
}

pub(super) fn verify(
    paths: &SnapshotPaths,
    manifest: &Manifest,
    state: &Sections,
) -> Result<(), SnapshotError> {
    let memory = artifacts::digest_of(Artifact::Memory, &paths.memory())?;
    manifest
        .header()
        .memory
        .verify_generation(memory, file_len(Artifact::Memory, &paths.memory())?)?;
    let overlay = artifacts::digest_of(Artifact::Overlay, &paths.overlay())?;
    match state.devices[Slot::Overlay.index() as usize].specific() {
        DeviceSpecific::Block(block) if block.image_digest == overlay => Ok(()),
        _ => Err(SnapshotError::DeviceStateNotCanonical(Slot::Overlay)),
    }
}

fn file_len(artifact: Artifact, path: &Path) -> Result<u64, SnapshotError> {
    std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .map_err(|error| SnapshotError::io(artifact, "stat", &error))
}
