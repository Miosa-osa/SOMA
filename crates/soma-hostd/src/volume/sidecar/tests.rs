use super::*;

fn sidecar() -> VolumeSidecar {
    VolumeSidecar {
        volume: VolumeId::new([7; 16]).expect("nonzero"),
        instance: InstanceId::new([9; 16]).expect("nonzero"),
        generation: GenerationId::new([3; 32]).expect("nonzero"),
        tenant: "tenant-alpha".to_owned(),
        size_class_gib: 10,
        state: VolumeState::Attached,
        dek_id: [5; 16],
        created_at_nanos: 1_700_000_000_000_000_000,
        last_transition_nanos: 1_700_000_001_000_000_000,
    }
}

#[test]
fn a_sidecar_round_trips_through_its_exact_length() {
    let encoded = sidecar().encode().expect("encodes");
    assert_eq!(encoded.len(), SIDECAR_LEN);
    assert_eq!(VolumeSidecar::decode(&encoded).expect("decodes"), sidecar());
}

#[test]
fn the_persistent_path_set_is_the_four_adr_paths_in_order() {
    assert_eq!(
        PERSISTENCE_PATHS,
        [
            "/home/sandbox",
            "/workspace",
            "/var/lib/docker",
            "/opt/user"
        ]
    );
}

#[test]
fn a_short_frame_is_refused() {
    let encoded = sidecar().encode().expect("encodes");
    assert_eq!(
        VolumeSidecar::decode(&encoded[..SIDECAR_LEN - 1]),
        Err(SidecarError::Length)
    );
}

#[test]
fn a_bad_magic_is_refused_before_the_checksum() {
    let mut encoded = sidecar().encode().expect("encodes");
    encoded[0] = b'X';
    // The magic is checked before the checksum, so a byte that breaks both is reported as a
    // magic mismatch rather than as corruption.
    assert_eq!(VolumeSidecar::decode(&encoded), Err(SidecarError::Magic));
}

#[test]
fn a_flipped_body_byte_fails_the_checksum() {
    let mut encoded = sidecar().encode().expect("encodes");
    encoded[40] ^= 0x01;
    assert_eq!(VolumeSidecar::decode(&encoded), Err(SidecarError::Checksum));
}

#[test]
fn an_unknown_version_is_refused() {
    let mut encoded = sidecar().encode().expect("encodes");
    encoded[8] = 0xff;
    let body_len = SIDECAR_LEN - 32;
    let checksum: [u8; 32] = Sha256::digest(&encoded[..body_len]).into();
    encoded[body_len..].copy_from_slice(&checksum);
    assert_eq!(VolumeSidecar::decode(&encoded), Err(SidecarError::Version));
}

#[test]
fn an_unknown_state_is_refused() {
    let mut encoded = sidecar().encode().expect("encodes");
    encoded[10] = 0x7f;
    let body_len = SIDECAR_LEN - 32;
    let checksum: [u8; 32] = Sha256::digest(&encoded[..body_len]).into();
    encoded[body_len..].copy_from_slice(&checksum);
    assert_eq!(VolumeSidecar::decode(&encoded), Err(SidecarError::State));
}

#[test]
fn an_empty_tenant_is_refused_at_encode() {
    let mut refused = sidecar();
    refused.tenant = String::new();
    assert_eq!(refused.encode(), Err(SidecarError::Tenant));
}

#[test]
fn an_over_long_tenant_is_refused_at_encode() {
    let mut refused = sidecar();
    refused.tenant = "t".repeat(MAX_TENANT_BYTES + 1);
    assert_eq!(refused.encode(), Err(SidecarError::Tenant));
}

#[test]
fn a_zero_identity_is_refused() {
    let mut encoded = sidecar().encode().expect("encodes");
    encoded[11..27].copy_from_slice(&[0; 16]);
    let body_len = SIDECAR_LEN - 32;
    let checksum: [u8; 32] = Sha256::digest(&encoded[..body_len]).into();
    encoded[body_len..].copy_from_slice(&checksum);
    assert_eq!(VolumeSidecar::decode(&encoded), Err(SidecarError::Identity));
}

#[test]
fn every_state_has_a_distinct_stable_code() {
    assert_eq!(VolumeState::Attached.code(), 1);
    assert_eq!(VolumeState::Detached.code(), 2);
    assert_eq!(VolumeState::Deleting.code(), 3);
    assert_eq!(VolumeState::from_code(4), None);
}
