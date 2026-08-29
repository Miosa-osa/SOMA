//! Reading and installing the complete state of vCPU 0.
//!
//! The order is the certified one: configuration first (CPUID and MSRs), then registers,
//! floating-point and extended state, then the local APIC, multiprocessing, and pending event
//! state. Nothing here runs a vCPU; every call happens while the thread that owns it is
//! outside `KVM_RUN`.

use kvm_bindings::{KVM_MAX_CPUID_ENTRIES, Msrs, kvm_msr_entry, kvm_xsave};
use kvm_ioctls::KvmNestedStateBuffer;
use kvm_ioctls::{Cap, Kvm, VcpuFd};

use super::{error::SnapshotError, profile::XSAVE_LIMIT};
use crate::snapshot::kvm_state::{
    CpuidEntries, Fpu, LapicState, MpState, MsrEntries, Regs, Sregs, VcpuEvents, VcpuState,
    VcpuStateParts, Xcrs, XsaveArea,
};

/// The fixed SOMA allowlist of model-specific registers a version 1 snapshot carries.
///
/// It is intersected with `KVM_GET_MSR_INDEX_LIST`, so a host that does not implement an
/// entry simply drops it instead of failing, while an entry outside this list can never enter
/// or leave a snapshot however many registers the host supports. Every entry is architectural
/// guest state that the pinned kernel actually uses: the system-call and sysenter
/// configuration, segment bases, the memory-type and misc-enable configuration, the timestamp
/// counter and its deadline timer, the speculation-control and extended-supervisor-state
/// configuration, and the KVM paravirtual clock, async-page-fault, steal-time, and
/// end-of-interrupt registers.
///
/// `IA32_XSS` is not optional. The guest kernel enables its supervisor extended-state
/// components at boot and every later `XRSTORS` names them; a machine restored with the
/// register back at zero takes a general-protection fault the first time a task returns to
/// user mode. `IA32_SPEC_CTRL` is not optional either: dropping it would silently restore a
/// machine with weaker speculation mitigations than the one that was captured.
const MSR_ALLOWLIST: [u32; 26] = [
    0x0000_0010, // IA32_TSC
    0x0000_003b, // IA32_TSC_ADJUST
    0x0000_0048, // IA32_SPEC_CTRL
    0x0000_0174, // IA32_SYSENTER_CS
    0x0000_0175, // IA32_SYSENTER_ESP
    0x0000_0176, // IA32_SYSENTER_EIP
    0x0000_01a0, // IA32_MISC_ENABLE
    0x0000_0277, // IA32_CR_PAT
    0x0000_06e0, // IA32_TSC_DEADLINE
    0x0000_0da0, // IA32_XSS
    0x4b56_4d00, // KVM_WALL_CLOCK_NEW
    0x4b56_4d01, // KVM_SYSTEM_TIME_NEW
    0x4b56_4d02, // KVM_ASYNC_PF_EN
    0x4b56_4d03, // KVM_STEAL_TIME
    0x4b56_4d04, // KVM_PV_EOI_EN
    0x4b56_4d05, // KVM_POLL_CONTROL
    0x4b56_4d06, // KVM_ASYNC_PF_INT
    0x4b56_4d07, // KVM_ASYNC_PF_ACK
    0xc000_0080, // EFER
    0xc000_0081, // STAR
    0xc000_0082, // LSTAR
    0xc000_0083, // CSTAR
    0xc000_0084, // SYSCALL_MASK
    0xc000_0100, // FS_BASE
    0xc000_0101, // GS_BASE
    0xc000_0102, // KERNEL_GS_BASE
];

/// The allowlisted registers this host implements, in allowlist order.
///
/// # Errors
///
/// Returns the `KVM_GET_MSR_INDEX_LIST` failure.
pub(super) fn supported_msrs(kvm: &Kvm) -> Result<Vec<u32>, SnapshotError> {
    let supported = kvm
        .get_msr_index_list()
        .map_err(|error| SnapshotError::ioctl("KVM_GET_MSR_INDEX_LIST", error))?;
    let supported = supported.as_slice();
    Ok(MSR_ALLOWLIST
        .into_iter()
        .filter(|index| supported.contains(index))
        .collect())
}

/// Reads every certified state group of `vcpu`.
///
/// # Errors
///
/// Returns the first KVM failure, a partial MSR read, or
/// [`SnapshotError::NestedStatePresent`] when the vCPU carries nested state.
pub(super) fn read(kvm: &Kvm, vcpu: &VcpuFd) -> Result<VcpuState, SnapshotError> {
    reject_nested_state(kvm, vcpu)?;
    let cpuid = vcpu
        .get_cpuid2(KVM_MAX_CPUID_ENTRIES)
        .map_err(|error| SnapshotError::ioctl("KVM_GET_CPUID2", error))?;
    let indexes = supported_msrs(kvm)?;
    let mut msrs = table(&indexes)?;
    let read = vcpu
        .get_msrs(&mut msrs)
        .map_err(|error| SnapshotError::ioctl("KVM_GET_MSRS", error))?;
    if read != indexes.len() {
        return Err(SnapshotError::PartialTable {
            operation: "KVM_GET_MSRS",
            expected: indexes.len(),
            actual: read,
        });
    }
    let sregs = vcpu
        .get_sregs()
        .map_err(|error| SnapshotError::ioctl("KVM_GET_SREGS", error))?;
    let xsave = vcpu
        .get_xsave()
        .map_err(|error| SnapshotError::ioctl("KVM_GET_XSAVE", error))?;
    let lapic = vcpu
        .get_lapic()
        .map_err(|error| SnapshotError::ioctl("KVM_GET_LAPIC", error))?;
    Ok(VcpuState::new(VcpuStateParts {
        cpuid: CpuidEntries::try_from(&cpuid)?,
        msrs: MsrEntries::try_from(&msrs)?,
        regs: Regs::from(
            vcpu.get_regs()
                .map_err(|error| SnapshotError::ioctl("KVM_GET_REGS", error))?,
        ),
        sregs: Sregs::try_from(sregs)?,
        fpu: Fpu::from(
            vcpu.get_fpu()
                .map_err(|error| SnapshotError::ioctl("KVM_GET_FPU", error))?,
        ),
        xcrs: Xcrs::try_from(
            vcpu.get_xcrs()
                .map_err(|error| SnapshotError::ioctl("KVM_GET_XCRS", error))?,
        )?,
        xsave: XsaveArea::from(&xsave),
        lapic: LapicState::from(&lapic),
        mp_state: MpState::try_from(
            vcpu.get_mp_state()
                .map_err(|error| SnapshotError::ioctl("KVM_GET_MP_STATE", error))?,
        )?,
        events: VcpuEvents::try_from(
            vcpu.get_vcpu_events()
                .map_err(|error| SnapshotError::ioctl("KVM_GET_VCPU_EVENTS", error))?,
        )?,
        nested: None,
    })?)
}

/// Installs the CPU configuration: the CPUID template and the allowlisted MSRs.
///
/// # Errors
///
/// Returns the KVM failure or a partial MSR write.
pub(super) fn write_configuration(
    _kvm: &Kvm,
    vcpu: &VcpuFd,
    state: &VcpuState,
) -> Result<(), SnapshotError> {
    vcpu.set_cpuid2(&state.cpuid().try_into()?)
        .map_err(|error| SnapshotError::ioctl("KVM_SET_CPUID2", error))?;
    let msrs = Msrs::try_from(state.msrs())?;
    let written = vcpu
        .set_msrs(&msrs)
        .map_err(|error| SnapshotError::ioctl("KVM_SET_MSRS", error))?;
    if written == state.msrs().entries().len() {
        Ok(())
    } else {
        Err(SnapshotError::PartialTable {
            operation: "KVM_SET_MSRS",
            expected: state.msrs().entries().len(),
            actual: written,
        })
    }
}

/// Installs the register, floating-point, extended, local APIC, multiprocessing, and pending
/// event state, in the certified order.
///
/// # Errors
///
/// Returns the first KVM failure or [`SnapshotError::XsaveTooLarge`].
pub(super) fn write_registers(
    kvm: &Kvm,
    vcpu: &VcpuFd,
    state: &VcpuState,
) -> Result<(), SnapshotError> {
    vcpu.set_sregs(&(*state.sregs()).into())
        .map_err(|error| SnapshotError::ioctl("KVM_SET_SREGS", error))?;
    vcpu.set_regs(&(*state.regs()).into())
        .map_err(|error| SnapshotError::ioctl("KVM_SET_REGS", error))?;
    vcpu.set_fpu(&(*state.fpu()).into())
        .map_err(|error| SnapshotError::ioctl("KVM_SET_FPU", error))?;
    vcpu.set_xcrs(&state.xcrs().into())
        .map_err(|error| SnapshotError::ioctl("KVM_SET_XCRS", error))?;
    write_xsave(kvm, vcpu, &kvm_xsave::try_from(state.xsave())?)?;
    vcpu.set_lapic(&state.lapic().into())
        .map_err(|error| SnapshotError::ioctl("KVM_SET_LAPIC", error))?;
    vcpu.set_mp_state(state.mp_state().into())
        .map_err(|error| SnapshotError::ioctl("KVM_SET_MP_STATE", error))?;
    vcpu.set_vcpu_events(&(*state.events()).into())
        .map_err(|error| SnapshotError::ioctl("KVM_SET_VCPU_EVENTS", error))
}

/// Installs the 4,096-byte extended state, after proving the host cannot read past it.
fn write_xsave(kvm: &Kvm, vcpu: &VcpuFd, xsave: &kvm_xsave) -> Result<(), SnapshotError> {
    let reported = kvm.check_extension_int(Cap::Xsave2);
    if reported > XSAVE_LIMIT {
        return Err(SnapshotError::XsaveTooLarge(reported));
    }
    // SAFETY: `KVM_SET_XSAVE` copies the vCPU's user-ABI extended-state size from the
    // pointer. The check above proves the host reports no more than the 4,096 bytes a
    // `kvm_xsave` provides, and `kvm_xsave` is exactly that size, so the kernel cannot read
    // beyond the value borrowed here. The value outlives the call.
    unsafe { vcpu.set_xsave(xsave) }.map_err(|error| SnapshotError::ioctl("KVM_SET_XSAVE", error))
}

/// Refuses a vCPU that carries nested-virtualization state.
///
/// Version 1 does not certify nested state, so a machine that somehow entered guest mode is
/// rejected at capture rather than restored with silently dropped state.
fn reject_nested_state(kvm: &Kvm, vcpu: &VcpuFd) -> Result<(), SnapshotError> {
    if !kvm.check_extension(Cap::NestedState) {
        return Ok(());
    }
    let mut buffer = KvmNestedStateBuffer::empty();
    match vcpu.nested_state(&mut buffer) {
        Ok(None) => Ok(()),
        Ok(Some(_)) => Err(SnapshotError::NestedStatePresent),
        Err(error) => Err(SnapshotError::ioctl("KVM_GET_NESTED_STATE", error)),
    }
}

fn table(indexes: &[u32]) -> Result<Msrs, SnapshotError> {
    let entries: Vec<kvm_msr_entry> = indexes
        .iter()
        .map(|index| kvm_msr_entry {
            index: *index,
            reserved: 0,
            data: 0,
        })
        .collect();
    Msrs::from_entries(&entries).map_err(|_| SnapshotError::PartialTable {
        operation: "KVM_GET_MSRS",
        expected: indexes.len(),
        actual: 0,
    })
}
