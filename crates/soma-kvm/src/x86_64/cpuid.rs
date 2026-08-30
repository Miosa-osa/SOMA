//! The versioned SOMA CPU template applied over KVM's supported CPUID set.
//!
//! Version 1 keeps KVM's supported leaves, requires the KVM paravirtual signature leaf so the
//! guest selects `kvmclock`, pins the bootstrap vCPU's APIC identifiers to zero, and marks the
//! hypervisor bit. Anything the host cannot provide fails closed before vCPU execution.
//!
//! Several leaves report properties of whichever host processor answered the ioctl rather than
//! properties of the host: on a hybrid processor the two topology leaves carry that core's
//! x2APIC identifier and the cache leaves carry that core type's geometry, so the same host
//! answers the same question differently from one call to the next. Version 1 exposes one vCPU
//! with no topology and certifies no host cache geometry, so every such field is pinned. That
//! makes the template digest reproducible, which is what lets a snapshot be rejected for a
//! genuinely different CPU instead of for the scheduler's choice of core.

use kvm_bindings::{CpuId, KVM_MAX_CPUID_ENTRIES};
use kvm_ioctls::{Kvm, VcpuFd};

use super::error::{MachineError, Phase};

const LEAF_FEATURES: u32 = 0x1;
const LEAF_CACHE: u32 = 0x4;
const LEAF_TOPOLOGY: u32 = 0xb;
const LEAF_TOPOLOGY_V2: u32 = 0x1f;
const LEAF_L2_CACHE: u32 = 0x8000_0006;
const LEAF_KVM_SIGNATURE: u32 = 0x4000_0000;
const FEATURES_ECX_HYPERVISOR: u32 = 1 << 31;
const FEATURES_EBX_APIC_ID_MASK: u32 = 0xff << 24;
/// Cache type, level, self-initialising, and fully-associative bits; everything above them
/// counts cores and threads and is pinned to one.
const CACHE_EAX_KEPT: u32 = 0x0000_3fff;
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
            LEAF_CACHE => {
                entry.eax &= CACHE_EAX_KEPT;
                entry.ebx = 0;
                entry.ecx = 0;
            }
            LEAF_TOPOLOGY | LEAF_TOPOLOGY_V2 => entry.edx = 0,
            LEAF_L2_CACHE => {
                entry.ecx = 0;
                entry.edx = 0;
            }
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
    fn cache_geometry_and_sharing_counts_are_pinned() {
        let mut cpuid = CpuId::from_entries(&[
            kvm_cpuid_entry2 {
                function: LEAF_CACHE,
                index: 0,
                eax: 0xfc00_4121,
                ebx: 0x02c0_003f,
                ecx: 0x7f,
                ..kvm_cpuid_entry2::default()
            },
            entry(
                LEAF_KVM_SIGNATURE,
                KVM_SIGNATURE[0],
                KVM_SIGNATURE[1],
                KVM_SIGNATURE[2],
            ),
        ])
        .unwrap();
        apply_template(&mut cpuid).unwrap();
        let cache = cpuid.as_slice()[0];
        assert_eq!(cache.eax, 0x0121, "cache type and level must survive");
        assert_eq!((cache.ebx, cache.ecx), (0, 0));
    }

    #[test]
    fn template_pins_apic_ids_sets_hypervisor_bit_and_requires_signature() {
        let mut cpuid = CpuId::from_entries(&[
            entry(LEAF_FEATURES, 0x0700_0800, 0, 0),
            entry(LEAF_TOPOLOGY, 0, 0, 5),
            entry(LEAF_TOPOLOGY_V2, 0, 0, 0x28),
            entry(LEAF_L2_CACHE, 0, 0x1000_8040, 0x10),
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
