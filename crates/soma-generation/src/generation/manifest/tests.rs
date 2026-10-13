//! The durable data volume group, proved through the canonical codec.

use super::{STEP_A_PERSISTENCE_PATHS, decode_manifest, encode_manifest, fixture};

#[test]
fn policy_four_round_trips_with_its_persistence_group() {
    let manifest = fixture::profile_v4();
    let bytes = encode_manifest(&manifest).expect("encodes");
    let decoded = decode_manifest(&bytes).expect("decodes");
    assert_eq!(decoded, manifest);
    assert_eq!(decoded.persistence_paths().len(), 4);
    let paths: Vec<&str> = decoded
        .persistence_paths()
        .iter()
        .map(String::as_str)
        .collect();
    assert_eq!(paths, STEP_A_PERSISTENCE_PATHS);
}

#[test]
fn a_policy_below_four_never_writes_the_group() {
    // The version 3 fixture carries the field disabled, and the group is not written, so the
    // bytes are exactly what policy 3 always produced.
    let manifest = fixture::profile_v3();
    let bytes = encode_manifest(&manifest).expect("encodes");
    assert_eq!(decode_manifest(&bytes).expect("decodes"), manifest);
    assert!(
        decode_manifest(&bytes)
            .expect("decodes")
            .persistence_paths()
            .is_empty()
    );

    // Enabling it in memory under policy 3 must not change the encoding at all.
    let mut enabled = manifest.clone();
    enabled.persistence.enabled = true;
    enabled.persistence.paths = vec!["/workspace".to_owned()];
    assert_eq!(
        encode_manifest(&enabled).expect("encodes"),
        encode_manifest(&manifest).expect("encodes")
    );
}

#[test]
fn a_persistent_generation_declares_the_volume_slot_and_names_it() {
    let manifest = fixture::profile_v4();
    assert!(manifest.device_set().volume());
    let line = String::from_utf8(manifest.command_line.clone()).expect("utf8");
    assert!(line.contains("soma.volume=/dev/vdc"));
    // The volume page follows the memory device's, so no earlier slot's address moved.
    assert!(line.contains("4K@0xd0006000:11:6"));

    // A Generation below policy 4 never declares the slot and never names the device.
    let below = fixture::profile_v3();
    assert!(!below.device_set().volume());
    let line = String::from_utf8(below.command_line.clone()).expect("utf8");
    assert!(!line.contains("soma.volume="));
    assert!(!line.contains("0xd0006000"));
}

mod pinned;

#[test]
fn the_persistence_group_carries_the_class_the_volume_is_created_at() {
    use super::{DEFAULT_VOLUME_CLASS_GIB, VOLUME_CLASSES_GIB, is_volume_class};

    let manifest = fixture::profile_v4();
    assert!(manifest.persistence.enabled);
    assert_eq!(
        manifest.persistence.size_class_gib,
        DEFAULT_VOLUME_CLASS_GIB
    );
    let bytes = encode_manifest(&manifest).expect("encodes");
    let decoded = decode_manifest(&bytes).expect("decodes");
    assert_eq!(
        decoded.persistence.size_class_gib, DEFAULT_VOLUME_CLASS_GIB,
        "the class a capture, a restore, and every launch agree about survives the codec"
    );

    // Every class the contract fixes is a class the group may carry, and one it does not is
    // refused in both directions rather than written and read back as something else.
    for class in VOLUME_CLASSES_GIB {
        let mut candidate = manifest.clone();
        candidate.persistence.size_class_gib = class;
        assert!(is_volume_class(class));
        assert_eq!(
            decode_manifest(&encode_manifest(&candidate).expect("encodes"))
                .expect("decodes")
                .persistence
                .size_class_gib,
            class
        );
    }
    for refused in [0, 1, 11, 64] {
        let mut candidate = manifest.clone();
        candidate.persistence.size_class_gib = refused;
        assert!(
            encode_manifest(&candidate).is_err(),
            "a class the contract does not fix is refused, not written"
        );
    }
}
