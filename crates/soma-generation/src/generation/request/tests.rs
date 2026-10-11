use super::{CompilerProfile, ProfileLimits};
use soma_kvm::MachineContract;

#[test]
fn each_profile_declares_the_bounds_of_its_own_contract() {
    for contract in MachineContract::ALL {
        let limits = ProfileLimits::for_version(contract.version())
            .expect("every contract this implementation builds has a profile");
        assert_eq!(limits.max_vcpus, contract.max_vcpus());
        assert_eq!(
            limits.max_memory_mib * 1024 * 1024,
            contract.max_memory_bytes()
        );
    }
    assert_eq!(ProfileLimits::for_version(4), None);
    assert_eq!(CompilerProfile::v1().machine_contract, MachineContract::V1);
    assert_eq!(CompilerProfile::v2().machine_contract, MachineContract::V2);
    assert!(CompilerProfile::v1().validate().is_ok());
    assert!(CompilerProfile::v2().validate().is_ok());
    assert_eq!(CompilerProfile::v3().machine_contract, MachineContract::V3);
    assert!(CompilerProfile::v3().validate().is_ok());
}

#[test]
fn a_host_admits_version_three_now_that_the_machine_builds_its_device() {
    assert_eq!(
        CompilerProfile::from_policy_version(2),
        Some(CompilerProfile::v2())
    );
    assert_eq!(
        CompilerProfile::from_policy_version(3),
        Some(CompilerProfile::v3())
    );
    assert_eq!(CompilerProfile::from_policy_version(4), None);
}

#[test]
fn a_profile_whose_contract_and_policy_version_disagree_is_refused() {
    let mut profile = CompilerProfile::v1();
    profile.machine_contract = MachineContract::V2;
    assert!(profile.validate().is_err());
}

#[test]
fn the_v3_profile_is_a_live_resize_policy_so_its_kernel_must_carry_the_memory_driver() {
    use crate::generation::kernel_config::LIVE_RESIZE_POLICY_VERSION;
    assert!(CompilerProfile::v3().policy_version >= LIVE_RESIZE_POLICY_VERSION);
    assert!(CompilerProfile::v2().policy_version < LIVE_RESIZE_POLICY_VERSION);
}

#[test]
fn a_hotplug_region_belongs_to_the_profile_whose_machine_has_the_memory_device() {
    use super::{DEFAULT_HOTPLUG_BYTES, HOTPLUG_BLOCK_BYTES};
    assert_eq!(CompilerProfile::v1().hotplug_bytes, 0);
    assert_eq!(CompilerProfile::v2().hotplug_bytes, 0);
    assert_eq!(CompilerProfile::v3().hotplug_bytes, DEFAULT_HOTPLUG_BYTES);
    assert!(
        CompilerProfile::v2()
            .with_hotplug_bytes(HOTPLUG_BLOCK_BYTES)
            .validate()
            .is_err()
    );
    assert!(
        CompilerProfile::v3()
            .with_hotplug_bytes(0)
            .validate()
            .is_err()
    );
    assert!(
        CompilerProfile::v3()
            .with_hotplug_bytes(HOTPLUG_BLOCK_BYTES + 1)
            .validate()
            .is_err()
    );
    assert!(
        CompilerProfile::v3()
            .with_hotplug_bytes(HOTPLUG_BLOCK_BYTES)
            .validate()
            .is_ok()
    );
}
