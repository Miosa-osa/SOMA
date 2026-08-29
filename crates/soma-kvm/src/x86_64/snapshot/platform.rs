//! VM-level machine state: the in-kernel interrupt controller, the timer, the GSI routing
//! SOMA owns, and the paravirtual clock.
//!
//! Reads happen only while vCPU 0 is outside `KVM_RUN`; writes happen only before it resumes.

use kvm_bindings::{
    KVM_IRQCHIP_IOAPIC, KVM_IRQCHIP_PIC_MASTER, KVM_IRQCHIP_PIC_SLAVE, kvm_clock_data, kvm_irqchip,
    kvm_irqchip__bindgen_ty_1, kvm_pic_state,
};
use kvm_ioctls::VmFd;

use super::error::SnapshotError;
use crate::snapshot::kvm_state::{
    ClockState, IoapicState, IrqRoutingState, IrqchipState, PitState,
};

/// Reads the master PIC, the slave PIC, and the IOAPIC in that fixed order.
///
/// # Errors
///
/// Returns the first `KVM_GET_IRQCHIP` failure.
pub(super) fn read_irqchip(vm: &VmFd) -> Result<IrqchipState, SnapshotError> {
    Ok(IrqchipState {
        master: read_pic(vm, KVM_IRQCHIP_PIC_MASTER)?,
        slave: read_pic(vm, KVM_IRQCHIP_PIC_SLAVE)?,
        ioapic: read_ioapic(vm)?,
    })
}

/// Installs the master PIC, the slave PIC, and the IOAPIC in the same fixed order.
///
/// # Errors
///
/// Returns the first `KVM_SET_IRQCHIP` failure.
pub(super) fn write_irqchip(vm: &VmFd, state: &IrqchipState) -> Result<(), SnapshotError> {
    write_chip(
        vm,
        KVM_IRQCHIP_PIC_MASTER,
        &kvm_irqchip__bindgen_ty_1 {
            pic: kvm_pic_state::from(state.master),
        },
    )?;
    write_chip(
        vm,
        KVM_IRQCHIP_PIC_SLAVE,
        &kvm_irqchip__bindgen_ty_1 {
            pic: kvm_pic_state::from(state.slave),
        },
    )?;
    write_chip(
        vm,
        KVM_IRQCHIP_IOAPIC,
        &kvm_irqchip__bindgen_ty_1 {
            ioapic: (&state.ioapic).into(),
        },
    )
}

fn read_chip(vm: &VmFd, chip_id: u32) -> Result<kvm_irqchip, SnapshotError> {
    let mut chip = kvm_irqchip {
        chip_id,
        ..Default::default()
    };
    vm.get_irqchip(&mut chip)
        .map_err(|error| SnapshotError::ioctl("KVM_GET_IRQCHIP", error))?;
    Ok(chip)
}

fn read_pic(
    vm: &VmFd,
    chip_id: u32,
) -> Result<crate::snapshot::kvm_state::PicState, SnapshotError> {
    let chip = read_chip(vm, chip_id)?;
    // SAFETY: KVM filled the union's `pic` member because the request named a PIC chip id,
    // and every member is plain integer data for which all bit patterns are valid.
    Ok(unsafe { chip.chip.pic }.into())
}

fn read_ioapic(vm: &VmFd) -> Result<IoapicState, SnapshotError> {
    let chip = read_chip(vm, KVM_IRQCHIP_IOAPIC)?;
    // SAFETY: KVM filled the union's `ioapic` member because the request named the IOAPIC
    // chip id, and every member is plain integer data for which all bit patterns are valid.
    let ioapic = unsafe { chip.chip.ioapic };
    Ok((&ioapic).into())
}

fn write_chip(
    vm: &VmFd,
    chip_id: u32,
    chip: &kvm_irqchip__bindgen_ty_1,
) -> Result<(), SnapshotError> {
    let irqchip = kvm_irqchip {
        chip_id,
        pad: 0,
        chip: *chip,
    };
    vm.set_irqchip(&irqchip)
        .map_err(|error| SnapshotError::ioctl("KVM_SET_IRQCHIP", error))
}

/// Reads the in-kernel i8254 state.
///
/// # Errors
///
/// Returns the `KVM_GET_PIT2` failure.
pub(super) fn read_pit(vm: &VmFd) -> Result<PitState, SnapshotError> {
    vm.get_pit2()
        .map(PitState::from)
        .map_err(|error| SnapshotError::ioctl("KVM_GET_PIT2", error))
}

/// Installs the in-kernel i8254 state.
///
/// # Errors
///
/// Returns the `KVM_SET_PIT2` failure.
pub(super) fn write_pit(vm: &VmFd, state: PitState) -> Result<(), SnapshotError> {
    vm.set_pit2(&state.into())
        .map_err(|error| SnapshotError::ioctl("KVM_SET_PIT2", error))
}

/// Reads the paravirtual clock.
///
/// # Errors
///
/// Returns the `KVM_GET_CLOCK` failure or an unknown clock flag.
pub(super) fn read_clock(vm: &VmFd) -> Result<ClockState, SnapshotError> {
    let clock = vm
        .get_clock()
        .map_err(|error| SnapshotError::ioctl("KVM_GET_CLOCK", error))?;
    Ok(ClockState::try_from(clock)?)
}

/// Installs the paravirtual clock.
///
/// # Errors
///
/// Returns the `KVM_SET_CLOCK` failure.
pub(super) fn write_clock(vm: &VmFd, state: ClockState) -> Result<(), SnapshotError> {
    vm.set_clock(&kvm_clock_data::from(state))
        .map_err(|error| SnapshotError::ioctl("KVM_SET_CLOCK", error))
}

/// The GSI routing SOMA owns, which version 1 leaves empty.
///
/// KVM has no routing read ioctl, so the section records the overrides SOMA installed rather
/// than a table read back from the kernel. The machine installs none: the in-kernel irqchip
/// creates default routes for GSIs 0 through 23 and the five device lines use them, so an
/// empty section means "the certified default routing" and any future override becomes a
/// visible, compared difference.
#[must_use]
pub(super) fn owned_routing() -> IrqRoutingState {
    IrqRoutingState::default()
}

/// Installs the recorded routing overrides, if the snapshot carries any.
///
/// # Errors
///
/// Returns the conversion or `KVM_SET_GSI_ROUTING` failure.
pub(in crate::x86_64) fn write_routing(
    vm: &VmFd,
    state: &IrqRoutingState,
) -> Result<(), SnapshotError> {
    if state.entries().is_empty() {
        return Ok(());
    }
    let mut routing = kvm_bindings::KvmIrqRouting::new(state.entries().len()).map_err(|_| {
        SnapshotError::PartialTable {
            operation: "KVM_SET_GSI_ROUTING",
            expected: state.entries().len(),
            actual: 0,
        }
    })?;
    for (slot, entry) in routing.as_mut_slice().iter_mut().zip(state.entries()) {
        *slot = (*entry).into();
    }
    vm.set_gsi_routing(&routing)
        .map_err(|error| SnapshotError::ioctl("KVM_SET_GSI_ROUTING", error))
}
