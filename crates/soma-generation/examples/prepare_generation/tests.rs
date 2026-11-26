//! The shape rules this tool applies before it builds anything.

use soma_generation::CompilerProfile;

use super::persistence_from;
use super::{Args, BuildInputs, DEFAULT_MEMORY_MIB, DEFAULT_STORAGE_MIB, MIB, admit_storage};

fn args(vcpus: u16, memory_mib: u64, storage_mib: u64) -> Args {
    Args {
        reference: "node:22".to_owned(),
        inputs: BuildInputs {
            layout: "layout".into(),
            kernel: "kernel".into(),
            kernel_config: "config".into(),
            agent: "agent".into(),
            erofs_tools: "erofs".into(),
            e2fsprogs: "e2fsprogs".into(),
            out_entry: "entry".into(),
            ceiling_mib: None,
        },
        vcpus,
        memory_mib,
        storage_mib,
        network: false,
    }
}

/// SOMA#150: the default writable size was 10240 MiB, which version 1 (the default shape's
/// profile) does not list, so the documented first Generation could never be certified.
#[test]
fn the_default_writable_size_is_one_every_profile_admits() {
    let default = args(1, DEFAULT_MEMORY_MIB, DEFAULT_STORAGE_MIB);
    for profile in [
        CompilerProfile::v1(),
        CompilerProfile::v2(),
        CompilerProfile::v3(),
        CompilerProfile::v4(),
    ] {
        assert_eq!(
            admit_storage(&profile, &default),
            Ok(()),
            "version {}",
            profile.policy_version
        );
    }
    assert!(
        CompilerProfile::v1()
            .overlay_capacities
            .contains(&(DEFAULT_STORAGE_MIB * MIB))
    );
}

#[test]
fn an_inadmissible_size_is_refused_naming_the_sizes_that_are() {
    let refused = admit_storage(&CompilerProfile::v1(), &args(1, 1024, 10240))
        .expect_err("version 1 has no 10 GiB class");
    assert!(refused.contains("storage_mib 10240"), "{refused}");
    assert!(refused.contains("version 1"), "{refused}");
    assert!(
        refused.contains("it admits 256, 1024, 4096 MiB"),
        "{refused}"
    );
    assert_eq!(
        admit_storage(&CompilerProfile::v3(), &args(1, 1024, 10240)),
        Ok(()),
        "the live-resize machine lists 10 GiB"
    );
}

/// The scripts that build or launch the default entry state the same default, or a launch of
/// the entry they built is refused for a storage size that is not the one it was built at.
#[test]
fn the_scripts_default_to_the_tools_writable_size() {
    let prepare = include_str!("../../../../scripts/prepare-generation.sh");
    let reproduce = include_str!("../../../../scripts/reproduce.sh");
    let end_to_end = include_str!("../../../../scripts/end-to-end-check.sh");
    assert!(prepare.contains(&format!("DISK_MIB=\"${{5:-{DEFAULT_STORAGE_MIB}}}\"")));
    assert!(reproduce.contains(&format!("DISK_MIB={DEFAULT_STORAGE_MIB}\n")));
    assert!(end_to_end.contains(&format!("STORAGE_MIB={DEFAULT_STORAGE_MIB}\n")));
}

#[test]
fn the_persistence_policy_is_asked_for_rather_than_implied() {
    // Absent, blank, and the two false spellings are all "no"; the two true spellings are
    // "yes"; anything else is refused rather than read as one of them.
    for value in [None, Some(""), Some("0"), Some("false")] {
        assert_eq!(persistence_from(value), Ok(false), "value {value:?}");
    }
    for value in [Some("1"), Some("true")] {
        assert_eq!(persistence_from(value), Ok(true), "value {value:?}");
    }
    assert!(persistence_from(Some("yes")).is_err());
}

#[test]
fn a_desktop_is_always_captured_with_its_durable_volume() {
    // A computer keeps its files across stop and start, so every desktop Generation is the
    // persistent machine (compiler profile 4) and no caller can capture one without it.
    let desktop = include_str!("../../../../scripts/desktop-generation.sh");
    assert!(desktop.contains("SOMA_PREPARE_PERSISTENCE=1 \\\n"));
    assert!(!desktop.contains("SOMA_PREPARE_PERSISTENCE=\"$"));
}
