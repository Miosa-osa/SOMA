use super::{record::tests::record, *};
use crate::{InstanceId, Phase, TransferStep};

#[test]
fn appends_are_durable_sequenced_and_replayed_in_order() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = Ledger::open(dir.path()).expect("ledger");
    assert_eq!(
        ledger
            .append(&record(RecordKind::Constructing, 1))
            .expect("append"),
        1
    );
    assert_eq!(
        ledger
            .append(&record(RecordKind::Sterile, 1))
            .expect("append"),
        2
    );
    let reopened = Ledger::open(dir.path()).expect("reopen");
    assert_eq!(
        reopened
            .append(&record(RecordKind::Claiming, 1).detail(1))
            .expect("append"),
        3
    );
    let records = reopened.records().expect("records");
    assert_eq!(records.len(), 3);
    assert_eq!(records[2].1.kind, RecordKind::Claiming);
    let entries = reopened.entries().expect("entries");
    let entry = &entries[&record(RecordKind::Sterile, 1).worker];
    assert_eq!(entry.phase, Phase::Claiming);
    assert_eq!(entry.records, 3);
    assert!(entry.operation.is_some());
    let claims = reopened.claims().expect("claims");
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].class, 1);
}

#[test]
fn corrupt_and_contended_records_fail_closed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = Ledger::open(dir.path()).expect("ledger");
    ledger
        .append(&record(RecordKind::Constructing, 1))
        .expect("append");
    fs::write(dir.path().join("0000000000000001.rec"), b"junk").expect("overwrite");
    assert_eq!(ledger.records(), Err(LedgerError::Corrupt { seq: 1 }));
    assert_eq!(ledger.entries(), Err(LedgerError::Corrupt { seq: 1 }));
    fs::write(dir.path().join("0000000000000002.rec"), b"foreign").expect("foreign");
    assert_eq!(
        ledger.append(&record(RecordKind::Sterile, 1)),
        Err(LedgerError::Contended { seq: 2 })
    );
}

#[test]
fn projection_rejects_sterile_after_assignment_and_records_after_death() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = Ledger::open(dir.path()).expect("ledger");
    let worker = record(RecordKind::Constructing, 1).worker;
    for kind in [
        RecordKind::Constructing,
        RecordKind::Sterile,
        RecordKind::Claiming,
        RecordKind::TransferStep,
    ] {
        ledger.append(&record(kind, 1).detail(1)).expect("append");
    }
    ledger
        .append(&record(RecordKind::Assigned, 1).instance(InstanceId::new([9; 16]).expect("id")))
        .expect("append");
    let entry = ledger.entries().expect("entries")[&worker].clone();
    assert!(entry.was_assigned);
    assert_eq!(entry.last_step, Some(TransferStep::Identity));
    assert_eq!(entry.phase, Phase::Assigned);
    ledger
        .append(&record(RecordKind::Sterile, 1))
        .expect("append");
    assert_eq!(
        ledger.entries(),
        Err(LedgerError::Invariant {
            worker,
            kind: RecordKind::Sterile,
        })
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = Ledger::open(dir.path()).expect("ledger");
    ledger
        .append(&record(RecordKind::Sterile, 2))
        .expect("append");
    assert!(matches!(
        ledger.entries(),
        Err(LedgerError::Invariant {
            kind: RecordKind::Sterile,
            ..
        })
    ));
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = Ledger::open(dir.path()).expect("ledger");
    for kind in [
        RecordKind::Constructing,
        RecordKind::ConstructFailed,
        RecordKind::Sterile,
    ] {
        ledger.append(&record(kind, 3)).expect("append");
    }
    assert!(matches!(
        ledger.entries(),
        Err(LedgerError::Invariant {
            kind: RecordKind::Sterile,
            ..
        })
    ));
}
