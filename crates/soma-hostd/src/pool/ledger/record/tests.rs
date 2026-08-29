//! Round trip and corruption proofs for the fixed-layout ledger record.

use super::*;

pub(crate) fn record(kind: RecordKind, worker: u8) -> Record {
    record_at(kind, worker, LeaseGeneration::FIRST)
}

/// The generation a worker holds once a claim has bumped it.
pub(crate) fn claimed() -> LeaseGeneration {
    LeaseGeneration::FIRST.next().expect("second generation")
}

pub(crate) fn record_at(kind: RecordKind, worker: u8, generation: LeaseGeneration) -> Record {
    Record::new(
        kind,
        WorkerId::new([worker; 16]).expect("worker"),
        generation,
        PoolKeyDigest::from_bytes([5; 32]),
    )
    .operation(OperationId::new([2; 16]).expect("operation"))
    .fingerprint(RequestFingerprint::of(b"x"))
    .identity(WorkerIdentity {
        process: 7,
        token: [8; 16],
    })
}

#[test]
fn records_round_trip_and_every_bit_flip_is_rejected() {
    let sample = record(RecordKind::Claiming, 1).detail(3);
    let encoded = sample.encode();
    assert_eq!(Record::decode(&encoded), Some(sample));
    for index in 0..RECORD_LEN {
        let mut flipped = encoded;
        flipped[index] ^= 0x01;
        assert_eq!(Record::decode(&flipped), None, "byte {index}");
    }
    assert_eq!(Record::decode(&encoded[..RECORD_LEN - 1]), None);
    let mut long = encoded.to_vec();
    long.push(0);
    assert_eq!(Record::decode(&long), None);
    assert_eq!(RecordKind::from_code(0), None);
    assert!(now_nanos() > 0);
}
