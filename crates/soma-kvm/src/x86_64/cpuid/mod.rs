//! The versioned SOMA CPU template applied over KVM's supported CPUID set.
//!
//! Version 1 keeps KVM's supported leaves, requires the KVM paravirtual signature leaf so the
//! guest selects `kvmclock`, pins the APIC identifiers to the processor's own index, and marks
//! the hypervisor bit. Anything the host cannot provide fails closed before vCPU execution.
//!
//! The identifier matters as soon as there is more than one processor. `KVM_GET_SUPPORTED_CPUID`
//! answers with the APIC identifier of whichever host processor serviced the call, so it is
//! pinned; for a single-vCPU machine that pin is zero, which is what version 1 has always
//! installed, and for a multi-vCPU machine it is the index KVM assigned in creation order, which
//! is the identifier the machine's MP table lists.
//!
//! Several leaves report properties of whichever host processor answered the ioctl rather than
//! properties of the host: on a hybrid processor the two topology leaves carry that core's
//! x2APIC identifier and the cache leaves carry that core type's geometry, so the same host
//! answers the same question differently from one call to the next. Version 1 exposes one vCPU
//! with no topology and certifies no host cache geometry, so every such field is pinned. That
//! makes the template digest reproducible, which is what lets a snapshot be rejected for a
//! genuinely different CPU instead of for the scheduler's choice of core.
//!
//! Version 2 admits eight processors, and a machine with several processors cannot leave the
//! leaves that describe them alone. The host's answers describe the host: a package that holds
//! several logical processors, caches shared between hyper-threads it pairs, and a processor
//! count that is the host's own. This machine runs one thread per core in one node, so version 2
//! states each of those fields from the machine it actually is, which is what makes the CPUID
//! topology agree with the MP table the guest finds its processors from.
//!
//! It also states the two features a host may not offer on its own. The deadline timer is stated
//! rather than inherited because on this path KVM's answer describes the silicon and not the
//! machine: KVM decides which local-APIC timer modes a guest may select from the guest's own
//! CPUID alone (`kvm_vcpu_after_set_cpuid`, which widens `timer_mode_mask` to every mode only
//! when the guest carries the bit) and then arms a software timer for the mode the guest selects
//! (`start_sw_tscdeadline`). A host that does not enumerate the feature therefore has no bearing
//! on the emulation, and `KVM_SET_CPUID2` accepts the bit outside its supported set, which is
//! what makes stating it possible at all. The second is the bit that says a package holds more
//! than one logical processor.
//!
//! What version 2 leaves alone it leaves on purpose. The legacy logical-processor count
//! `CPUID.1:EBX[23:16]` stays as the host answered it, because a version 2 guest decides its
//! topology from the MP table and the extended topology leaves and never from that field. The
//! legacy L2 and L3 geometry `CPUID.80000006` stays pinned to zero, because the sizes the guest
//! reads are the deterministic-cache leaves' and this one is a host property either way.

use kvm_bindings::{CpuId, KVM_MAX_CPUID_ENTRIES};
use kvm_ioctls::{Kvm, VcpuFd};

use super::error::{MachineError, Phase};
use crate::contract::MachineContract;

#[cfg(test)]
mod tests;

const LEAF_FEATURES: u32 = 0x1;
const LEAF_CACHE: u32 = 0x4;
const LEAF_TOPOLOGY: u32 = 0xb;
const LEAF_TOPOLOGY_V2: u32 = 0x1f;
const LEAF_AMD_CACHE: u32 = 0x8000_001d;
const LEAF_AMD_TOPOLOGY: u32 = 0x8000_001e;
const LEAF_L2_CACHE: u32 = 0x8000_0006;
const LEAF_AMD_FEATURES: u32 = 0x8000_0008;
const LEAF_KVM_SIGNATURE: u32 = 0x4000_0000;
const FEATURES_ECX_HYPERVISOR: u32 = 1 << 31;
const FEATURES_EBX_APIC_ID_MASK: u32 = 0xff << 24;
/// `CPUID.1:ECX[24]`, the TSC deadline timer.
///
/// KVM offers it only when the host processor enumerates it, and the hosts this runs on answer
/// zero here: a guest without the bit arms every timer through the local APIC instead, which is
/// the slower path and the whole reason version 2 states it.
const FEATURES_ECX_TSC_DEADLINE: u32 = 1 << 24;
/// `CPUID.1:EDX[28]`, "this package holds more than one logical processor". A machine with more
/// than one processor sets it, which the host's own answer does not do for a guest.
const FEATURES_EDX_HTT: u32 = 1 << 28;
/// Cache type, level, self-initialising, and fully-associative bits; everything above them
/// counts cores and threads and is pinned to one.
const CACHE_EAX_KEPT: u32 = 0x0000_3fff;
/// `CPUID.8000001D:EAX[7:5]`, one cache's level.
const AMD_CACHE_LEVEL: u32 = 0x7 << AMD_CACHE_LEVEL_SHIFT;
/// Where one cache's level sits in `EAX`.
const AMD_CACHE_LEVEL_SHIFT: u32 = 5;
/// `CPUID.8000001D:EAX[25:14]`, the logical processors sharing one cache, less one.
const AMD_CACHE_SHARING: u32 = 0x3ff << AMD_CACHE_SHARING_SHIFT;
/// Where the sharing count sits in `EAX`.
const AMD_CACHE_SHARING_SHIFT: u32 = 14;
/// `CPUID.8000001E:EBX[15:0]`, one processor's compute unit and the threads in that unit.
const AMD_TOPOLOGY_UNIT: u32 = 0xffff;
/// `CPUID.8000001E:ECX[10:0]`, one processor's node and the nodes in its package.
const AMD_TOPOLOGY_NODE: u32 = 0x7ff;
/// `CPUID.80000008:ECX[7:0]`, the physical threads in one package, less one.
const AMD_FEATURES_THREADS: u32 = 0xff;
/// `KVMKVMKVM\0\0\0` split over `EBX`, `ECX`, and `EDX`.
const KVM_SIGNATURE: [u32; 3] = [0x4b4d_564b, 0x564b_4d56, 0x0000_004d];

/// The machine a template describes, as far as its processors are concerned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GuestMachine {
    contract: MachineContract,
    /// How many processors this machine has, at least one.
    vcpus: u16,
}

impl GuestMachine {
    /// Describes a `vcpus`-processor machine built under `contract`.
    #[must_use]
    pub(crate) const fn new(contract: MachineContract, vcpus: u16) -> Self {
        Self { contract, vcpus }
    }

    /// The single processor of a diagnostic boot.
    #[must_use]
    pub(crate) const fn one() -> Self {
        Self::new(MachineContract::V1, 1)
    }

    /// The contract this machine was built under.
    #[must_use]
    pub(crate) const fn contract(self) -> MachineContract {
        self.contract
    }

    /// Whether this machine states the leaves a host will not state for it.
    const fn states_its_own_processors(self) -> bool {
        matches!(self.contract, MachineContract::V2)
    }

    /// The processors in this machine, less one, which is how the leaves count them. A machine
    /// always has at least one, so this is the count the leaves state and never a wrapped one.
    const fn threads_less_one(self) -> u32 {
        (self.vcpus.saturating_sub(1)) as u32
    }
}

/// Installs the template for one processor of `machine`, identified as `apic_id`.
///
/// # Errors
///
/// Returns the KVM failure, or the rejection when the host cannot provide a leaf the template
/// requires.
pub(crate) fn install(
    kvm: &Kvm,
    vcpu: &VcpuFd,
    apic_id: u8,
    machine: GuestMachine,
) -> Result<(), MachineError> {
    let mut cpuid = kvm
        .get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)
        .map_err(|error| MachineError::os(Phase::Cpuid, error))?;
    apply_template(&mut cpuid, apic_id, machine)?;
    vcpu.set_cpuid2(&cpuid)
        .map_err(|error| MachineError::os(Phase::Cpuid, error))
}

/// Applies the template for one processor of `machine` to a supported CPUID set.
///
/// # Errors
///
/// Returns the rejection when the paravirtual signature leaf is absent or carries a signature
/// that is not KVM's.
pub(crate) fn apply_template(
    cpuid: &mut CpuId,
    apic_id: u8,
    machine: GuestMachine,
) -> Result<(), MachineError> {
    let mut signature_seen = false;
    let identifier = u32::from(apic_id) << 24;
    for entry in cpuid.as_mut_slice() {
        match entry.function {
            LEAF_FEATURES => {
                entry.ebx = (entry.ebx & !FEATURES_EBX_APIC_ID_MASK) | identifier;
                entry.ecx |= FEATURES_ECX_HYPERVISOR;
                if machine.states_its_own_processors() {
                    entry.ecx |= FEATURES_ECX_TSC_DEADLINE;
                    entry.edx |= FEATURES_EDX_HTT;
                }
            }
            LEAF_CACHE => {
                entry.eax &= CACHE_EAX_KEPT;
                entry.ebx = 0;
                entry.ecx = 0;
            }
            LEAF_TOPOLOGY | LEAF_TOPOLOGY_V2 => entry.edx = u32::from(apic_id),
            LEAF_AMD_CACHE => {
                if machine.states_its_own_processors() {
                    let sharing = amd_cache_sharing(entry.eax, machine) << AMD_CACHE_SHARING_SHIFT;
                    entry.eax = (entry.eax & !AMD_CACHE_SHARING) | sharing;
                }
            }
            LEAF_AMD_TOPOLOGY => {
                if machine.states_its_own_processors() {
                    // One thread per core in one node, so the compute unit is the processor, the
                    // threads in it are one, and the node is the only one. The host answers all
                    // three about the core that serviced the ioctl.
                    entry.eax = u32::from(apic_id);
                    entry.ebx = (entry.ebx & !AMD_TOPOLOGY_UNIT) | u32::from(apic_id);
                    entry.ecx &= !AMD_TOPOLOGY_NODE;
                }
            }
            LEAF_AMD_FEATURES => {
                if machine.states_its_own_processors() {
                    entry.ecx = (entry.ecx & !AMD_FEATURES_THREADS) | machine.threads_less_one();
                }
            }
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

/// The logical processors sharing one of this machine's caches, less one, as one cache states it.
///
/// A level 1 or level 2 cache belongs to the processor that asked, because this machine runs one
/// thread per core; the last level belongs to every processor in the package. A level the
/// template does not recognise keeps the sharing the host stated.
fn amd_cache_sharing(eax: u32, machine: GuestMachine) -> u32 {
    match (eax & AMD_CACHE_LEVEL) >> AMD_CACHE_LEVEL_SHIFT {
        1 | 2 => 0,
        3 => machine.threads_less_one(),
        _ => (eax & AMD_CACHE_SHARING) >> AMD_CACHE_SHARING_SHIFT,
    }
}
