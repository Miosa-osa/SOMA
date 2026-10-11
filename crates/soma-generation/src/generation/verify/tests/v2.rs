//! Machine contract version 2: the fixture is accepted under its own profile, refused by the
//! version 1 profile, and rejected when any field it binds is tampered with.

use super::{Incompatibility, MIB, require_profile};
use crate::generation::{
    artifacts::Sha256Digest,
    manifest::{GenerationManifest, fixture},
    request::CompilerProfile,
};

fn profile(version: u16) -> CompilerProfile {
    let mut profile = match version {
        3 => CompilerProfile::v3(),
        2 => CompilerProfile::v2(),
        _ => CompilerProfile::v1(),
    };
    profile.overlay_capacities = vec![256 * MIB, 1024 * MIB];
    profile
}

#[track_caller]
fn rejected(reason: Incompatibility, mutate: impl FnOnce(&mut GenerationManifest)) {
    let mut manifest = fixture::profile_v2();
    mutate(&mut manifest);
    let error = require_profile(&manifest, &profile(2)).expect_err("the mutation must be rejected");
    assert_eq!(error.incompatibility(), Some(reason));
}

#[test]
fn the_version_two_fixture_is_accepted_by_its_own_profile() {
    assert_eq!(
        require_profile(&fixture::profile_v2(), &profile(2))
            .map_err(|error| error.incompatibility()),
        Ok(())
    );
    // The version 1 profile admits one vCPU and three gigabytes, so the version 2 fixture is a
    // machine it cannot host and is refused for the policy version it binds.
    assert_eq!(
        require_profile(&fixture::profile_v2(), &profile(1))
            .map_err(|error| error.incompatibility()),
        Err(Some(Incompatibility::PolicyVersion))
    );
}

#[test]
fn the_version_one_fixture_is_still_accepted_by_the_version_one_profile() {
    assert_eq!(
        require_profile(&fixture::profile_v1(), &profile(1))
            .map_err(|error| error.incompatibility()),
        Ok(())
    );
}

#[test]
fn every_version_two_bound_field_rejects_on_mismatch() {
    rejected(Incompatibility::ContractStatement, |manifest| {
        manifest.machine_contract.digest = Sha256Digest::from_bytes([9; 32]);
    });
    rejected(Incompatibility::CommandLine, |manifest| {
        manifest.command_line = crate::generation::contracts::kernel_command_line_v1(
            crate::generation::manifest::fixture::devices(),
        );
    });
    rejected(Incompatibility::VcpuCount, |manifest| {
        manifest.shape.vcpu_count = 9;
    });
    rejected(Incompatibility::VcpuCount, |manifest| {
        manifest.shape.vcpu_count = 0;
    });
    rejected(Incompatibility::MemorySize, |manifest| {
        manifest.shape.memory_bytes = 32 * 1024 * MIB;
    });
    rejected(Incompatibility::MemorySlotVersion, |manifest| {
        manifest.shape.memory_slot_layout_version = 1;
    });
}

#[test]
fn an_unknown_machine_contract_version_fails_closed() {
    // Neither a command line nor a binding exists for version 4, so a manifest naming it matches
    // nothing and is refused rather than verified against some other contract's bytes.
    rejected(Incompatibility::CommandLine, |manifest| {
        manifest.machine_contract.version = 4;
    });
}

#[test]
fn the_version_three_fixture_is_accepted_only_by_its_own_profile() {
    assert_eq!(
        require_profile(&fixture::profile_v3(), &profile(3))
            .map_err(|error| error.incompatibility()),
        Ok(())
    );
    // Versions 1 and 2 refuse a manifest that binds policy version 3, and version 3 refuses theirs.
    for older in [1, 2] {
        assert_eq!(
            require_profile(&fixture::profile_v3(), &profile(older))
                .map_err(|error| error.incompatibility()),
            Err(Some(Incompatibility::PolicyVersion))
        );
    }
    assert_eq!(
        require_profile(&fixture::profile_v2(), &profile(3))
            .map_err(|error| error.incompatibility()),
        Err(Some(Incompatibility::PolicyVersion))
    );
}

#[test]
fn the_version_three_command_line_names_the_memory_device_and_version_two_does_not() {
    let v3 = String::from_utf8(fixture::profile_v3().command_line).expect("ASCII");
    let v2 = String::from_utf8(fixture::profile_v2().command_line).expect("ASCII");
    assert!(v3.contains("virtio_mmio.device=4K@0xd0005000:10:5"), "{v3}");
    assert!(!v2.contains("0xd0005000"), "{v2}");
}

#[test]
fn a_version_three_manifest_carrying_the_version_two_machine_is_refused() {
    let mut manifest = fixture::profile_v3();
    manifest.machine_contract = crate::generation::contracts::machine_contract_v2();
    manifest.machine_contract.version = 3;
    let error = require_profile(&manifest, &profile(3)).expect_err("a contract mix is refused");
    assert_eq!(
        error.incompatibility(),
        Some(Incompatibility::ContractStatement)
    );
}
