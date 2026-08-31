//! Decoding the manifest sections a restore needs.
//!
//! Every section is decoded into its typed value before any of it reaches a live VM, so a
//! malformed, short, or tampered section is a typed rejection rather than a partially
//! constructed machine.

use super::super::error::SnapshotError;
use crate::snapshot::{
    device_state::{DeviceSpecific, DeviceState},
    kvm_state::{ClockState, IrqRoutingState, IrqchipState, PitState, VcpuState, VmState},
    manifest::Manifest,
    section::SectionRole,
};
use crate::virtio::{DeviceSet, Slot};

/// Every decoded section a restore needs, in manifest order.
pub(super) struct Sections {
    pub(super) vm: VmState,
    pub(super) vcpu: VcpuState,
    pub(super) irqchip: IrqchipState,
    pub(super) routing: IrqRoutingState,
    pub(super) clock: ClockState,
    pub(super) pit: PitState,
    /// One decoded record per present slot, in table order.
    pub(super) devices: Vec<(Slot, DeviceState)>,
}

impl Sections {
    /// Decodes the sections for exactly the slots `devices` says this machine has.
    ///
    /// Compatibility has already refused a manifest whose section set disagrees with that, so a
    /// missing section here is a malformed snapshot rather than a smaller machine.
    pub(super) fn read(manifest: &Manifest, devices: DeviceSet) -> Result<Self, SnapshotError> {
        let devices = devices
            .present()
            .map(|slot| {
                let role = super::super::device::role(slot)
                    .ok_or(SnapshotError::DeviceStateNotCanonical(slot))?;
                let state = DeviceState::decode_for_slot(slot.index(), section(manifest, role)?)?;
                Ok((slot, state))
            })
            .collect::<Result<Vec<_>, SnapshotError>>()?;
        Ok(Self {
            vm: VmState::decode(section(manifest, SectionRole::VmState)?)?,
            vcpu: VcpuState::decode(section(manifest, SectionRole::Vcpu0)?)?,
            irqchip: IrqchipState::decode(section(manifest, SectionRole::Irqchip)?)?,
            routing: IrqRoutingState::decode(section(manifest, SectionRole::IrqRouting)?)?,
            clock: ClockState::decode(section(manifest, SectionRole::KvmClock)?)?,
            pit: PitState::decode(section(manifest, SectionRole::Pit)?)?,
            devices,
        })
    }
}

pub(super) fn section(manifest: &Manifest, role: SectionRole) -> Result<&[u8], SnapshotError> {
    manifest
        .section(role)
        .map(crate::snapshot::section::Section::payload)
        .ok_or(SnapshotError::MissingSection(match role {
            SectionRole::VmState => "VmState",
            SectionRole::Vcpu0 => "Vcpu0",
            SectionRole::Irqchip => "Irqchip",
            SectionRole::IrqRouting => "IrqRouting",
            SectionRole::KvmClock => "KvmClock",
            SectionRole::Pit => "Pit",
            SectionRole::RepairPointMarker => "RepairPointMarker",
            _ => "Device",
        }))
}

impl Sections {
    /// The decoded record for one slot, if this machine has it.
    pub(super) fn slot(&self, slot: Slot) -> Option<&DeviceState> {
        self.devices
            .iter()
            .find(|(present, _)| *present == slot)
            .map(|(_, state)| state)
    }
}

/// The placeholder MAC the snapshot carries, or the placeholder a machine with no network
/// device is built with: nothing ever reads it, because there is no device to install it on.
pub(super) fn net_mac(state: Option<&DeviceState>) -> Result<[u8; 6], SnapshotError> {
    match state.map(DeviceState::specific) {
        Some(DeviceSpecific::Net { mac, .. }) => Ok(mac),
        None => Ok([0; 6]),
        Some(_) => Err(SnapshotError::DeviceStateNotCanonical(Slot::Net)),
    }
}

pub(super) fn vsock_cid(state: Option<&DeviceState>) -> Result<u64, SnapshotError> {
    match state.map(DeviceState::specific) {
        Some(DeviceSpecific::Vsock { cid_placeholder }) => Ok(cid_placeholder),
        _ => Err(SnapshotError::DeviceStateNotCanonical(Slot::Vsock)),
    }
}
