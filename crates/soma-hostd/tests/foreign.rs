//! A grant belongs to the pool that issued it and to no other.

#![cfg(unix)]

mod support;

use soma_hostd::{DestroyReason, Phase, RecordKind, TransferFault, testing::ProcessTable};
use support::{admission, intent, ledger_dir, limits, op, open_with};

#[test]
fn a_grant_presented_to_another_pool_is_refused_and_poisons_no_ledger() {
    let admission = admission();
    let table = ProcessTable::new();
    let issuer_dir = ledger_dir();
    let other_dir = ledger_dir();
    let issuer = open_with(issuer_dir.path(), &table, limits(1, 2), &admission);
    let other = open_with(other_dir.path(), &table, limits(1, 2), &admission);
    issuer.replenish_blocking().expect("replenish");
    other.replenish_blocking().expect("replenish");
    let before = other.ledger().records().expect("records").len();

    let claim = issuer.claim(op(1), intent(1).fingerprint()).expect("claim");
    let worker = claim.outcome.worker;
    let failure = other
        .transfer(claim.grant.expect("grant"), &intent(1))
        .expect_err("a foreign grant is never admitted");
    assert_eq!(failure.worker, worker);
    assert_eq!(failure.fault, TransferFault::ForeignPool);
    assert_eq!(failure.step, None);
    assert!(failure.disposition.destroyed.complete);
    assert!(failure.disposition.released.complete);

    assert_eq!(
        other.ledger().records().expect("records").len(),
        before,
        "the receiving ledger recorded nothing about a worker it never constructed"
    );
    assert!(
        other.ledger().entries().is_ok(),
        "the receiving ledger still folds"
    );
    assert!(other.inspect(worker).is_none(), "no slot was adopted");
    assert_eq!(other.occupancy().sterile, 1, "no sterile worker was spent");

    assert_eq!(
        issuer.inspect(worker).map(|view| view.phase),
        Some(Phase::Dead),
        "the issuing pool destroyed its own worker"
    );
    assert!(issuer.release(worker).is_err());
    assert_eq!(
        issuer.ledger().entries().expect("entries")[&worker].phase,
        Phase::Dead
    );
    assert!(
        issuer
            .ledger()
            .records()
            .expect("records")
            .iter()
            .any(|(_, record)| record.kind == RecordKind::Destroying
                && record.detail == DestroyReason::ForeignPool as u8)
    );
    assert_eq!(issuer.broker().leased_heads(), 0);
    assert_eq!(issuer.broker().live_bundles(), 0);
    assert_eq!(
        admission.usage().residents,
        0,
        "the reservation was returned"
    );

    drop(other);
    let reopened = open_with(other_dir.path(), &table, limits(1, 2), &admission);
    assert_eq!(
        reopened
            .reconcile()
            .expect("the receiving pool still opens")
            .suspects,
        1
    );
}
