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
    assert_eq!(ProfileLimits::for_version(3), None);
    assert_eq!(CompilerProfile::v1().machine_contract, MachineContract::V1);
    assert_eq!(CompilerProfile::v2().machine_contract, MachineContract::V2);
    assert!(CompilerProfile::v1().validate().is_ok());
    assert!(CompilerProfile::v2().validate().is_ok());
}

#[test]
fn a_profile_whose_contract_and_policy_version_disagree_is_refused() {
    let mut profile = CompilerProfile::v1();
    profile.machine_contract = MachineContract::V2;
    assert!(profile.validate().is_err());
}
