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
use crate::virtio::{DeviceSet, MmioBus, Slot, SlotSnapshot};
use crate::x86_64::{
    Machine,
    devices::{self as machine_devices, DeviceIdentity},
};

/// The identity a restore installs: the snapshot's placeholder MAC, the context identifier it
/// was captured with, and the fresh one this Instance is assigned.
pub(super) struct Identity {
    pub(super) mac: [u8; 6],
    pub(super) captured_cid: u64,
}

pub(super) fn recreate_devices(
    machine: &Machine,
    root: std::fs::File,
    overlay_capacity_bytes: Option<u64>,
    state: &Sections,
    identity: &Identity,
    set: DeviceSet,
) -> Result<MmioBus, SnapshotError> {
    let captured_cid = u32::try_from(identity.captured_cid)
        .map_err(|_| SnapshotError::DeviceStateNotCanonical(Slot::Vsock))?;
    // Every restore builds the overlay slot against the head's declared shape and receives the
    // head itself at assignment, whether or not the caller already had one. One path means a
    // prepared worker and a direct restore cannot reach different device state.
    let devices = machine_devices::build_devices_detached_overlay(
        root,
        overlay_capacity_bytes,
        DeviceIdentity {
            guest_cid: captured_cid,
            guest_mac: identity.mac,
        },
        set,
    )?;
    let records = state
        .devices
        .iter()
        .map(|(slot, certified)| {
            Ok(SlotSnapshot {
                slot: *slot,
                transport: device::transport(certified),
                device: device::fresh_record(*slot, &devices, certified)?,
            })
        })
        .collect::<Result<Vec<_>, SnapshotError>>()?;
    // The device keeps the captured identifier here. The fresh one this Instance holds is
    // installed by the assignment step, which every restore goes through, so a prepared worker
    // built before its Instance exists and a restore that already knows its Instance cannot take
    // different paths to the same device state.
    let bus = MmioBus::restore(devices, &records, &machine.shared_ram())?;
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
    // A Generation with no writable storage published no overlay template, so there is no
    // sterile image to re-hash and nothing that could disagree with a record it never wrote.
    let Some(record) = state.slot(Slot::Overlay) else {
        return Ok(());
    };
    let overlay = artifacts::digest_of(Artifact::Overlay, &paths.overlay())?;
    match record.specific() {
        DeviceSpecific::Block(block) if block.image_digest == overlay => Ok(()),
        _ => Err(SnapshotError::DeviceStateNotCanonical(Slot::Overlay)),
    }
}

fn file_len(artifact: Artifact, path: &Path) -> Result<u64, SnapshotError> {
    std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .map_err(|error| SnapshotError::io(artifact, "stat", &error))
}
