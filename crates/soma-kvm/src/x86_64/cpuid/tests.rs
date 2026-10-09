use kvm_bindings::kvm_cpuid_entry2;

use super::*;

/// One leaf, with only the registers a test is about filled in.
fn cpuid_entry(
    function: u32,
    index: u32,
    eax: u32,
    ebx: u32,
    ecx: u32,
    edx: u32,
) -> kvm_cpuid_entry2 {
    kvm_cpuid_entry2 {
        function,
        index,
        eax,
        ebx,
        ecx,
        edx,
        ..kvm_cpuid_entry2::default()
    }
}

/// Every template is checked against this leaf, so every fixture carries it.
fn signature() -> kvm_cpuid_entry2 {
    cpuid_entry(
        LEAF_KVM_SIGNATURE,
        0,
        0,
        KVM_SIGNATURE[0],
        KVM_SIGNATURE[1],
        KVM_SIGNATURE[2],
    )
}

/// One leaf of an installed template.
///
/// `CpuId` keeps its entries in ascending leaf order, so a fixture is read by the leaf it means
/// rather than by where it happened to land.
fn leaf(cpuid: &CpuId, function: u32, index: u32) -> kvm_cpuid_entry2 {
    *cpuid
        .as_slice()
        .iter()
        .find(|entry| entry.function == function && entry.index == index)
        .expect("the fixture carries that leaf")
}

#[test]
fn cache_geometry_and_sharing_counts_are_pinned() {
    let cache = cpuid_entry(LEAF_CACHE, 0, 0xfc00_4121, 0x02c0_003f, 0x7f, 0);
    let mut cpuid = CpuId::from_entries(&[cache, signature()]).unwrap();
    apply_template(&mut cpuid, 0, GuestMachine::one()).unwrap();
    let cache = leaf(&cpuid, LEAF_CACHE, 0);
    assert_eq!(cache.eax, 0x0121, "cache type and level must survive");
    assert_eq!((cache.ebx, cache.ecx), (0, 0));
}

#[test]
fn template_pins_apic_ids_sets_hypervisor_bit_and_requires_signature() {
    let mut cpuid = CpuId::from_entries(&[
        cpuid_entry(LEAF_FEATURES, 0, 0, 0x0700_0800, 0, 0),
        cpuid_entry(LEAF_TOPOLOGY, 0, 0, 0, 0, 5),
        cpuid_entry(LEAF_TOPOLOGY_V2, 0, 0, 0, 0, 0x28),
        cpuid_entry(LEAF_L2_CACHE, 0, 0, 0, 0x1000_8040, 0x10),
        signature(),
    ])
    .unwrap();
    apply_template(&mut cpuid, 0, GuestMachine::one()).unwrap();
    assert_eq!(leaf(&cpuid, LEAF_FEATURES, 0).ebx, 0x0000_0800);
    assert_eq!(leaf(&cpuid, LEAF_FEATURES, 0).ecx, FEATURES_ECX_HYPERVISOR);
    assert_eq!(leaf(&cpuid, LEAF_TOPOLOGY, 0).edx, 0);
    assert_eq!(
        leaf(&cpuid, LEAF_TOPOLOGY_V2, 0).edx,
        0,
        "the v2 topology APIC id must be pinned too"
    );

    let mut without = CpuId::from_entries(&[cpuid_entry(LEAF_FEATURES, 0, 0, 0, 0, 0)]).unwrap();
    let error = apply_template(&mut without, 0, GuestMachine::one()).unwrap_err();
    assert_eq!(error.phase(), Phase::Cpuid);

    let mut wrong = CpuId::from_entries(&[cpuid_entry(LEAF_KVM_SIGNATURE, 0, 0, 1, 2, 3)]).unwrap();
    assert!(apply_template(&mut wrong, 0, GuestMachine::one()).is_err());
}

#[test]
fn version_one_states_nothing_the_host_did_not() {
    // Version 1 is certified as it is: whatever the host answers about its own package is what a
    // version 1 guest has always seen, and a template that started stating more would change the
    // digest every existing version 1 snapshot is pinned to. A host that pairs its threads must
    // survive untouched but for the identifier and the hypervisor bit.
    let mut cpuid = CpuId::from_entries(&[
        cpuid_entry(LEAF_FEATURES, 0, 0, 0x0700_0000, 0, 0),
        cpuid_entry(LEAF_AMD_CACHE, 0, 0x4021, 0, 0, 0),
        cpuid_entry(LEAF_AMD_TOPOLOGY, 0, 0x0102, 0x0100, 0x0777, 0),
        cpuid_entry(LEAF_AMD_FEATURES, 0, 0, 0, 0x501f, 0),
        signature(),
    ])
    .unwrap();
    apply_template(&mut cpuid, 3, GuestMachine::one()).unwrap();
    let features = leaf(&cpuid, LEAF_FEATURES, 0);
    assert_eq!(features.ebx, 0x0300_0000, "only the identifier moves");
    assert_eq!(features.ecx, FEATURES_ECX_HYPERVISOR);
    assert_eq!(features.edx, 0);
    assert_eq!(
        leaf(&cpuid, LEAF_AMD_CACHE, 0).eax,
        0x4021,
        "the host's cache sharing is not ours to state"
    );
    assert_eq!(
        leaf(&cpuid, LEAF_AMD_TOPOLOGY, 0).eax,
        0x0102,
        "nor is its compute unit"
    );
    assert_eq!(
        leaf(&cpuid, LEAF_AMD_FEATURES, 0).ecx,
        0x501f,
        "nor is its thread count"
    );
}

#[test]
fn version_two_states_its_own_processors_and_the_features_a_host_may_not_offer() {
    let mut cpuid = CpuId::from_entries(&[
        cpuid_entry(LEAF_FEATURES, 0, 0, 0x0700_0000, 0, 0),
        signature(),
    ])
    .unwrap();
    apply_template(&mut cpuid, 5, GuestMachine::new(MachineContract::V2, 8)).unwrap();
    let features = leaf(&cpuid, LEAF_FEATURES, 0);
    assert_eq!(
        features.ebx, 0x0500_0000,
        "the identifier is the vCPU's own"
    );
    assert_ne!(features.ecx & FEATURES_ECX_TSC_DEADLINE, 0);
    assert_ne!(features.edx & FEATURES_EDX_HTT, 0);
    assert_ne!(features.ecx & FEATURES_ECX_HYPERVISOR, 0);
}

#[test]
fn version_two_gives_every_processor_its_own_first_two_cache_levels() {
    // The host pairs hyper-threads, so it reports two logical processors sharing a level 1 and a
    // level 2 cache. This machine runs one thread per core, and eight of them in one package.
    let mut cpuid = CpuId::from_entries(&[
        cpuid_entry(LEAF_AMD_CACHE, 0, 0x4021, 0, 0, 0),
        cpuid_entry(LEAF_AMD_CACHE, 1, 0x4021, 0, 0, 0),
        cpuid_entry(LEAF_AMD_CACHE, 3, 0x4063, 0, 0, 0),
        signature(),
    ])
    .unwrap();
    apply_template(&mut cpuid, 0, GuestMachine::new(MachineContract::V2, 8)).unwrap();
    assert_eq!(
        leaf(&cpuid, LEAF_AMD_CACHE, 0).eax,
        0x0021,
        "a level 1 cache belongs to the core that asked"
    );
    assert_eq!(leaf(&cpuid, LEAF_AMD_CACHE, 1).eax, 0x0021);
    assert_eq!(
        leaf(&cpuid, LEAF_AMD_CACHE, 3).eax,
        0x1_c063,
        "the last level belongs to all eight"
    );
}

#[test]
fn version_two_places_every_processor_in_its_own_compute_unit() {
    let mut cpuid = CpuId::from_entries(&[
        cpuid_entry(LEAF_AMD_TOPOLOGY, 0, 0x0102, 0x0100, 0x0777, 0),
        cpuid_entry(LEAF_AMD_FEATURES, 0, 0, 0, 0x501f, 0),
        signature(),
    ])
    .unwrap();
    apply_template(&mut cpuid, 5, GuestMachine::new(MachineContract::V2, 8)).unwrap();
    let topology = leaf(&cpuid, LEAF_AMD_TOPOLOGY, 0);
    assert_eq!(
        topology.eax, 5,
        "the extended APIC id is this processor's own"
    );
    assert_eq!(
        topology.ebx, 0x0005,
        "one thread in one compute unit, which is this processor"
    );
    assert_eq!(topology.ecx, 0, "and one node for the whole package");
    assert_eq!(
        leaf(&cpuid, LEAF_AMD_FEATURES, 0).ecx,
        0x5007,
        "the package holds eight threads, whatever the host holds"
    );
}
