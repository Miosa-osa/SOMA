//! The versioned SOMA CPU template applied over KVM's supported CPUID set.
//!
//! Version 1 keeps KVM's supported leaves, requires the KVM paravirtual signature leaf so the
//! guest selects `kvmclock`, pins the bootstrap vCPU's APIC identifiers to zero, and marks the
//! hypervisor bit. Anything the host cannot provide fails closed before vCPU execution.
//!
//! Both topology leaves report the x2APIC identifier of whichever host processor answered the
//! ioctl, so both are pinned: leaving either as the host reported it would show the guest a
//! value that changes between calls on the same host and would make the certified template
//! digest unreproducible.

use kvm_bindings::{CpuId, KVM_MAX_CPUID_ENTRIES};
use kvm_ioctls::{Kvm, VcpuFd};

use super::error::{MachineError, Phase};

const LEAF_FEATURES: u32 = 0x1;
const LEAF_TOPOLOGY: u32 = 0xb;
const LEAF_TOPOLOGY_V2: u32 = 0x1f;
const LEAF_KVM_SIGNATURE: u32 = 0x4000_0000;
const FEATURES_ECX_HYPERVISOR: u32 = 1 << 31;
const FEATURES_EBX_APIC_ID_MASK: u32 = 0xff << 24;
/// `KVMKVMKVM\0\0\0` split over `EBX`, `ECX`, and `EDX`.
const KVM_SIGNATURE: [u32; 3] = [0x4b4d_564b, 0x564b_4d56, 0x0000_004d];

pub(crate) fn install(kvm: &Kvm, vcpu: &VcpuFd) -> Result<(), MachineError> {
    let mut cpuid = kvm
        .get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)
        .map_err(|error| MachineError::os(Phase::Cpuid, error))?;
    apply_template(&mut cpuid)?;
    vcpu.set_cpuid2(&cpuid)
        .map_err(|error| MachineError::os(Phase::Cpuid, error))
}

pub(crate) fn apply_template(cpuid: &mut CpuId) -> Result<(), MachineError> {
    let mut signature_seen = false;
    for entry in cpuid.as_mut_slice() {
        match entry.function {
            LEAF_FEATURES => {
                entry.ebx &= !FEATURES_EBX_APIC_ID_MASK;
                entry.ecx |= FEATURES_ECX_HYPERVISOR;
            }
            LEAF_TOPOLOGY | LEAF_TOPOLOGY_V2 => entry.edx = 0,
            LEAF_KVM_SIGNATURE => {
                signature_seen = true;
                if [entry.ebx, entry.ecx, entry.edx] != KVM_SIGNATURE {
                    return Err(MachineError::invalid(
                        Phase::Cpuid,
                        "KVM paravirtual signature leaf carries an unexpected signature",
                    ));
                }
            }
            _ => {}
        }
    }
    if !signature_seen {
        return Err(MachineError::invalid(
            Phase::Cpuid,
            "KVM paravirtual CPUID leaf 0x40000000 is missing",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use kvm_bindings::kvm_cpuid_entry2;

    use super::*;

    fn entry(function: u32, ebx: u32, ecx: u32, edx: u32) -> kvm_cpuid_entry2 {
        kvm_cpuid_entry2 {
            function,
            ebx,
            ecx,
            edx,
            ..kvm_cpuid_entry2::default()
        }
    }

    #[test]
    fn template_pins_apic_ids_sets_hypervisor_bit_and_requires_signature() {
        let mut cpuid = CpuId::from_entries(&[
            entry(LEAF_FEATURES, 0x0700_0800, 0, 0),
            entry(LEAF_TOPOLOGY, 0, 0, 5),
            entry(LEAF_TOPOLOGY_V2, 0, 0, 0x28),
            entry(
                LEAF_KVM_SIGNATURE,
                KVM_SIGNATURE[0],
                KVM_SIGNATURE[1],
                KVM_SIGNATURE[2],
            ),
        ])
        .unwrap();
        apply_template(&mut cpuid).unwrap();
        let entries = cpuid.as_slice();
        assert_eq!(entries[0].ebx, 0x0000_0800);
        assert_eq!(entries[0].ecx, FEATURES_ECX_HYPERVISOR);
        assert_eq!(entries[1].edx, 0);
        assert_eq!(
            entries[2].edx, 0,
            "the v2 topology APIC id must be pinned too"
        );

        let mut without = CpuId::from_entries(&[entry(LEAF_FEATURES, 0, 0, 0)]).unwrap();
        let error = apply_template(&mut without).unwrap_err();
        assert_eq!(error.phase(), Phase::Cpuid);

        let mut wrong = CpuId::from_entries(&[entry(LEAF_KVM_SIGNATURE, 1, 2, 3)]).unwrap();
        assert!(apply_template(&mut wrong).is_err());
    }
}
