//! Initial bootstrap vCPU state from the `x86_64` machine contract v1.
//!
//! The vCPU enters 32-bit protected mode with paging disabled, flat code and data segments, a
//! present 32-bit TSS, `RIP` at the entry point, and `RBX` pointing at `hvm_start_info`.

use kvm_bindings::{KVM_MAX_CPUID_ENTRIES, kvm_regs, kvm_segment, kvm_sregs};
use kvm_ioctls::{Kvm, VcpuFd};

use super::{
    error::{HaltGuestError, Phase},
    layout::START_INFO_ADDRESS,
};

const CR0_PE: u64 = 1;
const RFLAGS_RESERVED: u64 = 1 << 1;
const CODE_SELECTOR: u16 = 0x10;
const DATA_SELECTOR: u16 = 0x18;
const TSS_SELECTOR: u16 = 0x20;
const CODE_TYPE: u8 = 0xb;
const DATA_TYPE: u8 = 0x3;
const TSS_32_BUSY_TYPE: u8 = 0xb;
const TSS_LIMIT: u32 = 0x67;

pub(crate) fn install_cpuid(kvm: &Kvm, vcpu: &VcpuFd) -> Result<(), HaltGuestError> {
    let cpuid = kvm
        .get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)
        .map_err(|error| HaltGuestError::os(Phase::Cpuid, error))?;
    vcpu.set_cpuid2(&cpuid)
        .map_err(|error| HaltGuestError::os(Phase::Cpuid, error))
}

pub(crate) fn install_registers(vcpu: &VcpuFd, entry: u64) -> Result<(), HaltGuestError> {
    let mut sregs = vcpu
        .get_sregs()
        .map_err(|error| HaltGuestError::os(Phase::Sregs, error))?;
    apply_protected_mode(&mut sregs);
    vcpu.set_sregs(&sregs)
        .map_err(|error| HaltGuestError::os(Phase::Sregs, error))?;
    vcpu.set_regs(&boot_regs(entry))
        .map_err(|error| HaltGuestError::os(Phase::Regs, error))
}

pub(crate) fn apply_protected_mode(sregs: &mut kvm_sregs) {
    sregs.cs = flat_segment(CODE_SELECTOR, CODE_TYPE);
    sregs.ds = flat_segment(DATA_SELECTOR, DATA_TYPE);
    sregs.es = flat_segment(DATA_SELECTOR, DATA_TYPE);
    sregs.fs = flat_segment(DATA_SELECTOR, DATA_TYPE);
    sregs.gs = flat_segment(DATA_SELECTOR, DATA_TYPE);
    sregs.ss = flat_segment(DATA_SELECTOR, DATA_TYPE);
    sregs.tr = kvm_segment {
        base: 0,
        limit: TSS_LIMIT,
        selector: TSS_SELECTOR,
        type_: TSS_32_BUSY_TYPE,
        present: 1,
        dpl: 0,
        db: 0,
        s: 0,
        l: 0,
        g: 0,
        avl: 0,
        unusable: 0,
        padding: 0,
    };
    sregs.cr0 = CR0_PE;
    sregs.cr2 = 0;
    sregs.cr3 = 0;
    sregs.cr4 = 0;
    sregs.cr8 = 0;
    sregs.efer = 0;
    sregs.interrupt_bitmap = [0; 4];
}

pub(crate) fn boot_regs(entry: u64) -> kvm_regs {
    kvm_regs {
        rip: entry,
        rbx: START_INFO_ADDRESS,
        rflags: RFLAGS_RESERVED,
        ..kvm_regs::default()
    }
}

const fn flat_segment(selector: u16, type_: u8) -> kvm_segment {
    kvm_segment {
        base: 0,
        limit: 0xffff_ffff,
        selector,
        type_,
        present: 1,
        dpl: 0,
        db: 1,
        s: 1,
        l: 0,
        g: 1,
        avl: 0,
        unusable: 0,
        padding: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_mode_state_matches_the_contract() {
        let mut sregs = kvm_sregs {
            cr0: 0xffff,
            cr4: 0xffff,
            efer: 0xffff,
            ..kvm_sregs::default()
        };
        apply_protected_mode(&mut sregs);
        assert_eq!(sregs.cr0, CR0_PE);
        assert_eq!(sregs.cr4, 0);
        assert_eq!(sregs.efer, 0);
        assert_eq!(sregs.cs.type_, CODE_TYPE);
        assert_eq!(sregs.cs.limit, 0xffff_ffff);
        assert_eq!(
            (sregs.cs.db, sregs.cs.g, sregs.cs.s, sregs.cs.l),
            (1, 1, 1, 0)
        );
        for segment in [sregs.ds, sregs.es, sregs.fs, sregs.gs, sregs.ss] {
            assert_eq!(segment.type_, DATA_TYPE);
            assert_eq!(segment.base, 0);
            assert_eq!(segment.unusable, 0);
        }
        assert_eq!(sregs.tr.type_, TSS_32_BUSY_TYPE);
        assert_eq!(sregs.tr.limit, TSS_LIMIT);
        assert_eq!(sregs.tr.present, 1);
        assert_eq!(sregs.tr.s, 0);
    }

    #[test]
    fn boot_registers_carry_entry_and_start_info() {
        let regs = boot_regs(0x0100_0000);
        assert_eq!(regs.rip, 0x0100_0000);
        assert_eq!(regs.rbx, START_INFO_ADDRESS);
        assert_eq!(regs.rflags, RFLAGS_RESERVED);
        assert_eq!(
            (
                regs.rax, regs.rcx, regs.rdx, regs.rsp, regs.rbp, regs.rsi, regs.rdi
            ),
            (0, 0, 0, 0, 0, 0, 0)
        );
    }
}
