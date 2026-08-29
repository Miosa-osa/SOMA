use std::mem::{offset_of, size_of};

use kvm_bindings::{
    KVM_ARM_VCPU_PSCI_0_2, KVM_REG_ARM_CORE, KVM_REG_ARM64, KVM_REG_SIZE_U64, kvm_regs,
    kvm_vcpu_init, user_pt_regs,
};
use kvm_ioctls::{Error, VcpuFd, VmFd};

const PSTATE_EL1H_MASKED: u64 = 0x3c5;

pub(crate) fn initialize(
    vm: &VmFd,
    vcpu: &VcpuFd,
    kernel_entry: u64,
    fdt_address: u64,
) -> Result<(), Error> {
    let mut target = kvm_vcpu_init::default();
    vm.get_preferred_target(&mut target)?;
    target.features[0] |= 1_u32 << KVM_ARM_VCPU_PSCI_0_2;
    vcpu.vcpu_init(&target)?;

    set(vcpu, pc_id(), kernel_entry)?;
    set(vcpu, pstate_id(), PSTATE_EL1H_MASKED)?;
    set(vcpu, x_id(0), fdt_address)?;
    set(vcpu, x_id(1), 0)?;
    set(vcpu, x_id(2), 0)?;
    set(vcpu, x_id(3), 0)
}

fn set(vcpu: &VcpuFd, register: u64, value: u64) -> Result<(), Error> {
    vcpu.set_one_reg(register, &value.to_ne_bytes()).map(|_| ())
}

fn core_id(offset: usize) -> u64 {
    KVM_REG_ARM64
        | u64::from(KVM_REG_ARM_CORE)
        | KVM_REG_SIZE_U64
        | u64::try_from(offset / size_of::<u32>()).expect("register offset fits in u64")
}

fn x_id(index: usize) -> u64 {
    core_id(offset_of!(kvm_regs, regs) + offset_of!(user_pt_regs, regs) + index * size_of::<u64>())
}

fn pc_id() -> u64 {
    core_id(offset_of!(kvm_regs, regs) + offset_of!(user_pt_regs, pc))
}

fn pstate_id() -> u64 {
    core_id(offset_of!(kvm_regs, regs) + offset_of!(user_pt_regs, pstate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_register_ids_match_linux_kvm_abi_examples() {
        assert_eq!(x_id(0), 0x6030_0000_0010_0000);
        assert_eq!(pc_id(), 0x6030_0000_0010_0040);
        assert_eq!(pstate_id(), 0x6030_0000_0010_0042);
    }
}
