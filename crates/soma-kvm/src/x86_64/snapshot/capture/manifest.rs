//! Assembling one capture manifest out of the state the capture read.
//!
//! Nothing here touches KVM or the running machine. Every value it needs has already been read
//! and proved quiescent by the time it is called, so this is the one part of a capture with no
//! ordering requirement of its own: given the same parts it builds the same manifest.

use super::CaptureRequest;
use crate::contract::MachineContract;
use crate::snapshot::{
    Digest,
    device_state::DeviceState,
    kvm_state::VmState,
    manifest::{Architecture, CandidateId, Manifest, ManifestHeader, PageSize},
    memory::MemoryDescriptor,
    section::{Section, SectionRole},
};
use crate::virtio::Slot;
use crate::x86_64::snapshot::{device, error::SnapshotError, marker, profile};

pub(super) struct Parts<'a> {
    pub(super) contract: MachineContract,
    pub(super) cpu_template: Digest,
    pub(super) irqchip: &'a crate::snapshot::kvm_state::IrqchipState,
    pub(super) routing: &'a crate::snapshot::kvm_state::IrqRoutingState,
    pub(super) clock: &'a crate::snapshot::kvm_state::ClockState,
    pub(super) pit: &'a crate::snapshot::kvm_state::PitState,
    pub(super) devices: &'a [(Slot, DeviceState)],
    pub(super) device_set: crate::virtio::DeviceSet,
    pub(super) memory: MemoryDescriptor,
}

pub(super) fn build(
    request: &CaptureRequest<'_>,
    vm_state: &VmState,
    vcpu_states: &[crate::snapshot::kvm_state::VcpuState],
    parts: &Parts<'_>,
) -> Result<Manifest, SnapshotError> {
    let (Some(first), vcpus) = (
        vcpu_states.first(),
        u16::try_from(vcpu_states.len()).map_err(|_| {
            SnapshotError::NotQuiescent("the machine has more vCPUs than the format admits")
        })?,
    ) else {
        return Err(SnapshotError::NotQuiescent(
            "the machine captured no vCPU state",
        ));
    };
    // The slot count the restoring host must be able to provide comes from the same layout the
    // machine derived its ranges from, so a requirement and a layout cannot disagree.
    let layout = crate::memory_layout::GuestLayout::new(vm_state.total_bytes())
        .map_err(|error| SnapshotError::Machine(crate::x86_64::error::MachineError::from(error)))?;
    let slots = profile::required_memory_slots(layout);
    let header = ManifestHeader {
        architecture: Architecture::X86_64,
        page_size: PageSize::FOUR_KIB,
        candidate_id: CandidateId::new(request.candidate_id)?,
        machine_contract: profile::machine_contract(parts.contract, parts.device_set),
        device_contract: profile::device_contract(parts.device_set, parts.contract),
        cpu_template: parts.cpu_template,
        host: profile::requirements(slots)?,
        memory: parts.memory,
        vcpu_count: vcpus,
        guest_protocol_version: profile::GUEST_PROTOCOL_VERSION,
    };
    let mut sections = vec![
        Section::new(SectionRole::VmState, vm_state.encode())?,
        Section::new(SectionRole::Vcpu0, first.encode())?,
        Section::new(SectionRole::Irqchip, parts.irqchip.encode())?,
        Section::new(SectionRole::IrqRouting, parts.routing.encode())?,
        Section::new(SectionRole::KvmClock, parts.clock.encode())?,
        Section::new(SectionRole::Pit, parts.pit.encode())?,
    ];
    // The second and later processors take the codes immediately after the timer, so the
    // sequence stays ascending without renumbering a role version 1 already published.
    for (index, state) in vcpu_states.iter().enumerate().skip(1) {
        let role = SectionRole::vcpu(
            u16::try_from(index).map_err(|_| SnapshotError::NotQuiescent("vCPU index overflow"))?,
        )
        .ok_or(SnapshotError::NotQuiescent(
            "the machine has more vCPUs than the format admits",
        ))?;
        sections.push(Section::new(role, state.encode())?);
    }
    for (slot, state) in parts.devices {
        let role = device::role(*slot).ok_or(SnapshotError::DeviceStateNotCanonical(*slot))?;
        sections.push(Section::new(role, state.encode())?);
    }
    sections.push(Section::new(
        SectionRole::RepairPointMarker,
        marker::encode(&request.repair_point_line),
    )?);
    Ok(Manifest::new(header, sections)?)
}
