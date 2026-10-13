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
