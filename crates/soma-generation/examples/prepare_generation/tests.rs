//! The shape rules this tool applies before it builds anything.

use soma_generation::CompilerProfile;

use super::{Args, BuildInputs, DEFAULT_MEMORY_MIB, DEFAULT_STORAGE_MIB, MIB, admit_storage};
use super::{persistence_from, volume_class_from, with_volume_class};

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

/// Every Generation takes its volume unless the builder says otherwise (SOMA#262).
#[test]
fn the_volume_is_declared_unless_it_is_turned_off() {
    for value in [Some("0"), Some("false")] {
        assert_eq!(persistence_from(value), Ok(false), "value {value:?}");
    }
    for value in [None, Some(""), Some("1"), Some("true")] {
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

#[test]
fn a_persistent_generation_names_its_volume_class() {
    assert_eq!(volume_class_from(None), Ok(None));
    assert_eq!(volume_class_from(Some("")), Ok(None));
    for gib in [10, 20, 40] {
        assert_eq!(volume_class_from(Some(&gib.to_string())), Ok(Some(gib)));
    }
    for refused in ["0", "30", "80", "forty"] {
        assert!(volume_class_from(Some(refused)).is_err(), "{refused}");
    }

    let wide = with_volume_class(CompilerProfile::v4(), Some(40)).expect("a persistent class");
    assert_eq!(wide.persistence.size_class_gib, 40);
    assert_eq!(wide.policy_version, CompilerProfile::v4().policy_version);
    let fixed_wide = with_volume_class(CompilerProfile::v5().with_persistence(), Some(40))
        .expect("a fixed-memory persistent class");
    assert_eq!(fixed_wide.persistence.size_class_gib, 40);
    assert_eq!(
        fixed_wide.policy_version,
        CompilerProfile::v5().policy_version
    );
    let unstated = with_volume_class(CompilerProfile::v4(), None).expect("the profile's own");
    assert_eq!(unstated.persistence.size_class_gib, 10);
    // A class names the volume a Generation declares, so one without a volume cannot take it.
    assert!(with_volume_class(CompilerProfile::v3(), Some(40)).is_err());
}
