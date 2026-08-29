use super::{Digest, HEADER_LEN, MAX_SECTION_BYTES, Section, SectionError, SectionRole, Writer};
use crate::snapshot::WireError;
use crate::snapshot::wire::Reader;

#[test]
fn role_codes_are_unique_and_ordered() {
    let codes: Vec<u16> = SectionRole::ALL.iter().map(|role| role.code()).collect();
    let mut sorted = codes.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(codes, sorted);
    for role in SectionRole::ALL {
        assert_eq!(SectionRole::from_code(role.code()), Some(role));
    }
    assert_eq!(SectionRole::from_code(0x7fff), None);
    assert!(!SectionRole::Pit.is_required());
    assert_eq!(SectionRole::Device3.device_slot(), Some(3));
    assert_eq!(SectionRole::VmState.device_slot(), None);
}

#[test]
fn bounds_payload_length_at_construction() {
    let oversized = vec![0_u8; usize::try_from(MAX_SECTION_BYTES).unwrap() + 1];
    assert_eq!(
        Section::new(SectionRole::Pit, oversized).unwrap_err(),
        SectionError::PayloadTooLarge {
            length: u64::from(MAX_SECTION_BYTES) + 1
        }
    );
}

#[test]
fn skips_unknown_non_critical_and_rejects_unknown_critical() {
    let mut writer = Writer::default();
    Section::new(SectionRole::VmState, vec![1])
        .unwrap()
        .encode(&mut writer);
    writer.put_u16(0x7000);
    writer.put_u16(9);
    writer.put_u8(0);
    writer.put_u32(0);
    writer.put_bytes(Digest::of(&[]).as_bytes());
    let bytes = writer.finish();
    let mut reader = Reader::new(&bytes);
    let sections = Section::decode_sequence(&mut reader, 2).unwrap();
    assert_eq!(sections.len(), 1);
    assert_eq!(reader.finish(), Ok(()));

    let mut critical = bytes.clone();
    let flags_index = bytes.len() - 32 - 4 - 1;
    critical[flags_index] = 1;
    assert_eq!(
        Section::decode_sequence(&mut Reader::new(&critical), 2).unwrap_err(),
        SectionError::UnknownCriticalRole(0x7000)
    );
    critical[flags_index] = 2;
    assert_eq!(
        Section::decode_sequence(&mut Reader::new(&critical), 2).unwrap_err(),
        SectionError::ReservedFlags(2)
    );
}

#[test]
fn rejects_duplicate_role_and_digest_mismatch() {
    let mut writer = Writer::default();
    let section = Section::new(SectionRole::Vcpu0, vec![7, 7]).unwrap();
    section.encode(&mut writer);
    section.encode(&mut writer);
    let bytes = writer.finish();
    assert_eq!(
        Section::decode_sequence(&mut Reader::new(&bytes), 2).unwrap_err(),
        SectionError::RoleOrder {
            previous: 2,
            next: 2
        }
    );
    let mut corrupted = bytes;
    corrupted[HEADER_LEN] ^= 1;
    assert_eq!(
        Section::decode_sequence(&mut Reader::new(&corrupted[..corrupted.len() / 2]), 1),
        Err(SectionError::DigestMismatch { role: 2 })
    );
    assert_eq!(
        Section::decode_sequence(&mut Reader::new(&corrupted[..10]), 1),
        Err(SectionError::Wire(WireError::Truncated {
            needed: 32,
            available: 1
        }))
    );
}
