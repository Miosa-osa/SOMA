use super::*;

fn volume() -> VolumeId {
    VolumeId::new([7; 16]).expect("nonzero")
}

#[test]
fn a_record_round_trips_through_its_exact_length() {
    let record = VolumeRecord::new(VolumeRecordKind::AttachBegin, volume())
        .detail(0)
        .instance(InstanceId::new([9; 16]).expect("nonzero"))
        .operation(OperationId::new([1; 16]).expect("nonzero"))
        .fingerprint(RequestFingerprint::new([2; 32]).expect("nonzero"));
    let encoded = record.encode();
    assert_eq!(encoded.len(), VOLUME_RECORD_LEN);
    assert_eq!(VolumeRecord::decode(&encoded), Some(record));
}

#[test]
fn a_record_with_no_optional_parts_round_trips() {
    let record = VolumeRecord::new(VolumeRecordKind::DetachEnd, volume());
    assert_eq!(VolumeRecord::decode(&record.encode()), Some(record));
}

#[test]
fn a_flipped_byte_fails_the_checksum() {
    let mut encoded = VolumeRecord::new(VolumeRecordKind::Reaped, volume()).encode();
    encoded[30] ^= 0x01;
    assert_eq!(VolumeRecord::decode(&encoded), None);
}

#[test]
fn a_short_or_long_frame_is_refused() {
    let encoded = VolumeRecord::new(VolumeRecordKind::Repair, volume()).encode();
    assert_eq!(
        VolumeRecord::decode(&encoded[..VOLUME_RECORD_LEN - 1]),
        None
    );
    let mut longer = encoded.to_vec();
    longer.push(0);
    assert_eq!(VolumeRecord::decode(&longer), None);
}

#[test]
fn a_zero_volume_is_refused() {
    let mut encoded = VolumeRecord::new(VolumeRecordKind::CreateEnd, volume()).encode();
    encoded[15..31].copy_from_slice(&[0; 16]);
    let checksum: [u8; 32] = Sha256::digest(&encoded[..VOLUME_RECORD_BODY_LEN]).into();
    encoded[VOLUME_RECORD_BODY_LEN..].copy_from_slice(&checksum);
    assert_eq!(VolumeRecord::decode(&encoded), None);
}

#[test]
fn begin_and_end_pair_by_kind() {
    assert!(VolumeRecordKind::CreateBegin.is_begin());
    assert!(VolumeRecordKind::CreateEnd.is_end());
    assert!(!VolumeRecordKind::Repair.is_begin());
    assert!(!VolumeRecordKind::Repair.is_end());
    assert!(VolumeRecordKind::CreateEnd.closes(VolumeRecordKind::CreateBegin));
    assert!(!VolumeRecordKind::AttachEnd.closes(VolumeRecordKind::CreateBegin));
}
