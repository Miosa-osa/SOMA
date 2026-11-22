//! The Suspension header and member sealing: what must round-trip, and every shape of hostile or
//! damaged input that must not.

use super::seal::{CHUNK, DataKey, SealError, open_member, seal_member};
use super::*;

fn id(byte: u8) -> SuspensionId {
    SuspensionId::new([byte; 16]).expect("id")
}

fn header() -> Header {
    Header {
        id: id(1),
        generation: GenerationId::new([2; 32]).expect("generation"),
        host_profile: HostProfileDigest::new([3; 32]).expect("profile"),
        tenant: "tenant-a".to_owned(),
        shape: Shape {
            vcpus: 2,
            memory_mib: 4096,
            storage_mib: 20480,
        },
        created_unix: 1_000,
        expires_unix: 1_000 + 86_400,
        members: [
            MemberRecord {
                bytes: 4096,
                sha256: [4; 32],
            },
            MemberRecord {
                bytes: 8192,
                sha256: [5; 32],
            },
            MemberRecord {
                bytes: 512,
                sha256: [6; 32],
            },
        ],
    }
}

#[test]
fn a_header_round_trips_exactly() {
    let bytes = header().encode().expect("encode");
    assert_eq!(Header::decode(&bytes), Ok(header()));
}

#[test]
fn a_generation_snapshot_is_never_read_as_a_suspension() {
    let mut bytes = header().encode().expect("encode");
    bytes[..8].copy_from_slice(b"SOMASNP\0");
    assert_eq!(Header::decode(&bytes), Err(HeaderError::BadMagic));
}

#[test]
fn every_truncation_and_any_trailing_byte_is_refused() {
    let bytes = header().encode().expect("encode");
    for cut in 0..bytes.len() {
        assert!(Header::decode(&bytes[..cut]).is_err(), "cut at {cut}");
    }
    let mut longer = bytes;
    longer.push(0);
    assert_eq!(Header::decode(&longer), Err(HeaderError::Trailing));
}

#[test]
fn an_invalid_header_is_neither_written_nor_read() {
    let mut empty_tenant = header();
    empty_tenant.tenant.clear();
    assert_eq!(empty_tenant.encode(), Err(HeaderError::Tenant));

    let mut forever = header();
    forever.expires_unix = forever.created_unix + MAX_RETENTION_SECS + 1;
    assert_eq!(forever.encode(), Err(HeaderError::Lifetime));

    let mut backwards = header();
    backwards.expires_unix = backwards.created_unix;
    assert_eq!(backwards.encode(), Err(HeaderError::Lifetime));

    let mut no_cpu = header();
    no_cpu.shape.vcpus = 0;
    assert_eq!(no_cpu.encode(), Err(HeaderError::Shape));

    // A member role out of order is caught on read even though the writer never produces one.
    let mut bytes = header().encode().expect("encode");
    let first_member = bytes.len() - 3 * 41;
    bytes[first_member] = Member::Overlay as u8;
    assert_eq!(Header::decode(&bytes), Err(HeaderError::MemberOrder));
}

#[test]
fn expiry_is_a_fact_of_the_header() {
    let header = header();
    assert!(!header.expired_at(header.expires_unix - 1));
    assert!(header.expired_at(header.expires_unix));
}

fn sealed(key: &DataKey, plain: &[u8]) -> (Vec<u8>, MemberRecord) {
    let mut out = Vec::new();
    let record = seal_member(key, id(1), Member::Memory, &mut &plain[..], &mut out).expect("seal");
    (out, record)
}

fn open(
    key: &DataKey,
    member: Member,
    suspension: u8,
    record: &MemberRecord,
    bytes: &[u8],
) -> Result<Vec<u8>, SealError> {
    let mut out = Vec::new();
    open_member(
        key,
        id(suspension),
        member,
        record,
        &mut &bytes[..],
        &mut out,
    )
    .map(|()| out)
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len)
        .map(|index| u8::try_from(index * 31 % 251).expect("below 251"))
        .collect()
}

#[test]
fn members_round_trip_at_every_chunk_boundary() {
    let key = DataKey::from_bytes([9; 32]);
    for len in [0, 1, CHUNK - 1, CHUNK, CHUNK + 1, 2 * CHUNK, 2 * CHUNK + 7] {
        let plain = pattern(len);
        let (bytes, record) = sealed(&key, &plain);
        assert_eq!(record.bytes, len as u64);
        assert_eq!(
            open(&key, Member::Memory, 1, &record, &bytes).expect("open"),
            plain,
            "len {len}"
        );
    }
}

#[test]
fn a_wrong_key_another_member_or_another_suspension_does_not_open() {
    let key = DataKey::from_bytes([9; 32]);
    let (bytes, record) = sealed(&key, &pattern(CHUNK + 10));
    assert!(matches!(
        open(
            &DataKey::from_bytes([8; 32]),
            Member::Memory,
            1,
            &record,
            &bytes
        ),
        Err(SealError::Corrupt)
    ));
    assert!(matches!(
        open(&key, Member::Overlay, 1, &record, &bytes),
        Err(SealError::Corrupt)
    ));
    assert!(matches!(
        open(&key, Member::Memory, 2, &record, &bytes),
        Err(SealError::Corrupt)
    ));
}

#[test]
fn a_flipped_bit_a_swapped_chunk_a_cut_or_an_extension_is_refused() {
    let key = DataKey::from_bytes([9; 32]);
    let (bytes, record) = sealed(&key, &pattern(2 * CHUNK + 5));
    let sealed_chunk = CHUNK + 16;

    let mut flipped = bytes.clone();
    flipped[100] ^= 1;
    assert!(matches!(
        open(&key, Member::Memory, 1, &record, &flipped),
        Err(SealError::Corrupt)
    ));

    let mut swapped = bytes.clone();
    let (first, rest) = swapped.split_at_mut(sealed_chunk);
    first.swap_with_slice(&mut rest[..sealed_chunk]);
    assert!(matches!(
        open(&key, Member::Memory, 1, &record, &swapped),
        Err(SealError::Corrupt)
    ));

    // Dropping the final chunk leaves a full chunk that was not sealed as the last one.
    let cut = &bytes[..2 * sealed_chunk];
    assert!(open(&key, Member::Memory, 1, &record, cut).is_err());

    let mut extended = bytes.clone();
    extended.push(0);
    // The extra byte is read into the final chunk and breaks its tag; refused either way.
    assert!(open(&key, Member::Memory, 1, &record, &extended).is_err());

    // A whole extra chunk after a full final one is caught as an extension.
    let (exact, exact_record) = sealed(&key, &pattern(CHUNK));
    let mut longer = exact.clone();
    longer.extend_from_slice(&exact[..sealed_chunk]);
    assert!(matches!(
        open(&key, Member::Memory, 1, &exact_record, &longer),
        Err(SealError::Truncated)
    ));
}

#[test]
fn a_header_record_that_lies_about_the_member_is_refused() {
    let key = DataKey::from_bytes([9; 32]);
    let (bytes, record) = sealed(&key, &pattern(4096));
    let wrong_digest = MemberRecord {
        sha256: [0; 32],
        ..record
    };
    assert!(matches!(
        open(&key, Member::Memory, 1, &wrong_digest, &bytes),
        Err(SealError::Mismatch)
    ));
}
