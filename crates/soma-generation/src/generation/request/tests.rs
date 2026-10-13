use super::{CompilerProfile, ProfileLimits};
use crate::generation::manifest::{PersistenceBinding, STEP_A_PERSISTENCE_PATHS};
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
    // Version 4 is version 3's machine plus a declared device and no new machine, so its bounds
    // are version 3's rather than a version of their own.
    assert_eq!(
        ProfileLimits::for_version(4),
        ProfileLimits::for_version(MachineContract::V3.version())
    );
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
    assert_eq!(
        CompilerProfile::from_policy_version(4),
        Some(CompilerProfile::v4())
    );
}

#[test]
fn the_v4_profile_declares_the_durable_volume_over_version_threes_machine() {
    let profile = CompilerProfile::v4();
    assert!(profile.validate().is_ok());
    assert_eq!(profile.machine_contract, MachineContract::V3);
    assert_eq!(profile.hotplug_bytes, super::DEFAULT_HOTPLUG_BYTES);
    assert_eq!(
        profile.persistence,
        PersistenceBinding {
            enabled: true,
            size_class_gib: super::super::manifest::DEFAULT_VOLUME_CLASS_GIB,
            paths: STEP_A_PERSISTENCE_PATHS
                .iter()
                .map(|path| (*path).to_owned())
                .collect(),
        }
    );
    // A version this build knows is admitted, and no earlier profile changes at all.
    assert_eq!(
        CompilerProfile::from_policy_version(5),
        None,
        "a version with no profile is refused rather than admitted under the wrong one"
    );
    for earlier in [
        CompilerProfile::v1(),
        CompilerProfile::v2(),
        CompilerProfile::v3(),
    ] {
        assert!(
            !earlier.persistence.enabled,
            "profile {:?} must build no durable volume",
            earlier.policy_version
        );
        assert_eq!(earlier.persistence, PersistenceBinding::default());
    }
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

#[test]
fn version_3_admits_the_default_writable_class_and_version_2_does_not() {
    let ten_gib = 10 * 1024 * 1024 * 1024;
    assert!(CompilerProfile::v3().overlay_capacities.contains(&ten_gib));
    assert!(!CompilerProfile::v2().overlay_capacities.contains(&ten_gib));
    assert!(CompilerProfile::v3().validate().is_ok());
}

#[test]
fn version_3_admits_the_forty_gigabyte_class_and_older_profiles_do_not() {
    let forty_gib = 40 * 1024 * 1024 * 1024;
    assert!(
        CompilerProfile::v3()
            .overlay_capacities
            .contains(&forty_gib)
    );
    assert!(
        !CompilerProfile::v2()
            .overlay_capacities
            .contains(&forty_gib)
    );
    assert!(CompilerProfile::v3().validate().is_ok());
}
