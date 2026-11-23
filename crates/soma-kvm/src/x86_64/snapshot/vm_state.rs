//! Shared, read-only KVM state capture from a VM whose vCPUs have left `KVM_RUN`.

use kvm_ioctls::{Kvm, VcpuFd, VmFd};

use crate::{
    snapshot::{
        capture::CaptureStep,
        kvm_state::{
            ClockState, IrqRoutingState, IrqchipState, MemorySlot, PitState, VcpuState, VmState,
        },
    },
    x86_64::{
        error::MachineError,
        layout::{GuestLayout, TSS_ADDRESS},
    },
};

use super::{SnapshotError, platform, vcpu};

pub(in crate::x86_64) struct PausedVmState {
    pub(in crate::x86_64) vm: VmState,
    pub(in crate::x86_64) vcpus: Vec<VcpuState>,
    pub(in crate::x86_64) irqchip: IrqchipState,
    pub(in crate::x86_64) routing: IrqRoutingState,
    pub(in crate::x86_64) clock: ClockState,
    pub(in crate::x86_64) pit: PitState,
}

impl PausedVmState {
    /// Reads the canonical machine state in restore order.
    ///
    /// The caller must hold the paused machine view through all reads.
    pub(in crate::x86_64) fn read(
        kvm: &Kvm,
        vm: &VmFd,
        vcpus: &[&VcpuFd],
        ram_bytes: u64,
        mut record: impl FnMut(CaptureStep) -> Result<(), SnapshotError>,
    ) -> Result<Self, SnapshotError> {
        let layout = GuestLayout::new(ram_bytes).map_err(MachineError::from)?;
        // Restore derives the same layout instead of trusting artifact-controlled slot geometry.
        let slots = layout
            .regions()
            .iter()
            .map(|region| MemorySlot {
                slot: region.slot,
                guest_address: region.guest_start,
                size: region.size,
                memory_offset: region.host_offset,
            })
            .collect();
        let vm_state = VmState::new(slots, TSS_ADDRESS, 0)?;
        record(CaptureStep::ReadVmState)?;
        let vcpu_states = vcpus
            .iter()
            .map(|vcpu| vcpu::read(kvm, vcpu))
            .collect::<Result<Vec<_>, SnapshotError>>()?;
        record(CaptureStep::ReadVcpuState)?;
        let irqchip = platform::read_irqchip(vm)?;
        record(CaptureStep::ReadIrqchip)?;
        let routing = platform::owned_routing();
        record(CaptureStep::ReadIrqRouting)?;
        let clock = platform::read_clock(vm)?;
        record(CaptureStep::ReadClock)?;
        let pit = platform::read_pit(vm)?;
        record(CaptureStep::ReadPit)?;
        Ok(Self {
            vm: vm_state,
            vcpus: vcpu_states,
            irqchip,
            routing,
            clock,
            pit,
        })
    }
}
