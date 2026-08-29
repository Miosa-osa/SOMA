use kvm_bindings::{
    KVM_DEV_ARM_VGIC_CTRL_INIT, KVM_DEV_ARM_VGIC_GRP_ADDR, KVM_DEV_ARM_VGIC_GRP_CTRL,
    KVM_DEV_ARM_VGIC_GRP_NR_IRQS, KVM_VGIC_V3_ADDR_TYPE_DIST, KVM_VGIC_V3_ADDR_TYPE_REDIST,
    kvm_create_device, kvm_device_attr, kvm_device_type_KVM_DEV_TYPE_ARM_VGIC_V3,
};
use kvm_ioctls::{DeviceFd, Error, VmFd};

use super::layout::{GIC_DIST_BASE, GIC_REDIST_BASE};

const IRQ_COUNT: u32 = 64;

pub(crate) fn create(vm: &VmFd) -> Result<DeviceFd, Error> {
    let mut descriptor = kvm_create_device {
        type_: kvm_device_type_KVM_DEV_TYPE_ARM_VGIC_V3,
        fd: 0,
        flags: 0,
    };
    let device = vm.create_device(&mut descriptor)?;
    set_address(&device, KVM_VGIC_V3_ADDR_TYPE_DIST, GIC_DIST_BASE)?;
    set_address(&device, KVM_VGIC_V3_ADDR_TYPE_REDIST, GIC_REDIST_BASE)?;

    let irq_count = IRQ_COUNT;
    device.set_device_attr(&kvm_device_attr {
        group: KVM_DEV_ARM_VGIC_GRP_NR_IRQS,
        attr: 0,
        addr: pointer_address(&irq_count),
        flags: 0,
    })?;
    device.set_device_attr(&kvm_device_attr {
        group: KVM_DEV_ARM_VGIC_GRP_CTRL,
        attr: u64::from(KVM_DEV_ARM_VGIC_CTRL_INIT),
        addr: 0,
        flags: 0,
    })?;
    Ok(device)
}

fn set_address(device: &DeviceFd, kind: u32, address: u64) -> Result<(), Error> {
    device.set_device_attr(&kvm_device_attr {
        group: KVM_DEV_ARM_VGIC_GRP_ADDR,
        attr: u64::from(kind),
        addr: pointer_address(&address),
        flags: 0,
    })
}

fn pointer_address<T>(value: &T) -> u64 {
    u64::try_from(std::ptr::from_ref(value).addr()).expect("ARM64 pointer address fits in u64")
}
