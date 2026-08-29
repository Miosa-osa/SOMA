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
use crate::virtio::Slot;

/// Every decoded section a restore needs, in manifest order.
pub(super) struct Sections {
    pub(super) vm: VmState,
    pub(super) vcpu: VcpuState,
    pub(super) irqchip: IrqchipState,
    pub(super) routing: IrqRoutingState,
    pub(super) clock: ClockState,
    pub(super) pit: PitState,
    pub(super) devices: Vec<DeviceState>,
}

impl Sections {
    pub(super) fn read(manifest: &Manifest) -> Result<Self, SnapshotError> {
        let devices = [
            SectionRole::Device0,
            SectionRole::Device1,
            SectionRole::Device2,
            SectionRole::Device3,
            SectionRole::Device4,
        ]
        .into_iter()
        .zip(Slot::ALL)
        .map(|(role, slot)| {
            Ok(DeviceState::decode_for_slot(
                slot.index(),
                section(manifest, role)?,
            )?)
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

pub(super) fn net_mac(state: &DeviceState) -> Result<[u8; 6], SnapshotError> {
    match state.specific() {
        DeviceSpecific::Net { mac, .. } => Ok(mac),
        _ => Err(SnapshotError::DeviceStateNotCanonical(Slot::Net)),
    }
}

pub(super) fn vsock_cid(state: &DeviceState) -> Result<u64, SnapshotError> {
    match state.specific() {
        DeviceSpecific::Vsock { cid_placeholder } => Ok(cid_placeholder),
        _ => Err(SnapshotError::DeviceStateNotCanonical(Slot::Vsock)),
    }
}
