//! Binding gates for the readiness receipt.

use super::{
    Digest, ReadinessChallenge, ReadinessDemand, ReadinessReceipt, ReadinessRefusal,
    RestoredIdentity, SessionEvidence,
};

fn identity(snapshot: u8, launch: u8) -> RestoredIdentity {
    RestoredIdentity::new(
        Digest::from_bytes([snapshot; 32]),
        Digest::from_bytes([launch; 32]),
    )
}

fn session(instance: u8) -> SessionEvidence {
    SessionEvidence::new([instance; 16], [2; 16], [3; 32]).expect("a bound session")
}

fn challenge(seed: u8) -> ReadinessChallenge {
    ReadinessChallenge::adopt([seed; 32]).expect("a fresh challenge")
}

fn tag_of(receipt: &ReadinessReceipt) -> [u8; 32] {
    receipt.tag
}

#[test]
fn a_receipt_authenticates_only_against_its_own_challenge_and_identity() {
    let own = challenge(1);
    let demand = ReadinessDemand::new(&own, identity(7, 8));
    let receipt = demand.attest(&session(9));

    assert_eq!(own.accepts(&identity(7, 8), &receipt), Ok(()));
    for wrong in [identity(6, 8), identity(7, 9)] {
        assert_eq!(
            own.accepts(&wrong, &receipt),
            Err(ReadinessRefusal::Rejected),
            "a receipt completed a restore of another snapshot or launch authority"
        );
    }
    assert_eq!(
        challenge(2).accepts(&identity(7, 8), &receipt),
        Err(ReadinessRefusal::Rejected),
        "a receipt completed another Instance's restore"
    );
    assert_eq!(
        own.accepts(&identity(7, 8), &receipt.with_tag([0xaa; 32])),
        Err(ReadinessRefusal::Rejected),
        "a forged tag was accepted"
    );
}

#[test]
fn every_bound_session_field_changes_the_receipt() {
    let own = challenge(1);
    let demand = ReadinessDemand::new(&own, identity(7, 8));
    let receipt = demand.attest(&session(9));

    for other in [
        SessionEvidence::new([10; 16], [2; 16], [3; 32]).expect("another Instance"),
        SessionEvidence::new([9; 16], [4; 16], [3; 32]).expect("another operation"),
        SessionEvidence::new([9; 16], [2; 16], [5; 32]).expect("another transcript"),
    ] {
        let moved = demand.attest(&other);
        assert_ne!(
            tag_of(&moved),
            tag_of(&receipt),
            "a session field is not bound into the receipt"
        );
        assert_eq!(own.accepts(&identity(7, 8), &moved), Ok(()));
    }
    assert_eq!(receipt.session(), &session(9));
    assert_eq!(receipt.session().instance(), &[9; 16]);
    assert_eq!(receipt.session().operation(), &[2; 16]);
}

#[test]
fn an_unbound_session_and_an_empty_challenge_are_refused() {
    for unbound in [
        SessionEvidence::new([0; 16], [2; 16], [3; 32]),
        SessionEvidence::new([1; 16], [0; 16], [3; 32]),
        SessionEvidence::new([1; 16], [2; 16], [0; 32]),
    ] {
        assert_eq!(unbound, Err(ReadinessRefusal::Unbound));
    }
    assert_eq!(
        ReadinessChallenge::adopt([0; 32]).err(),
        Some(ReadinessRefusal::Unavailable)
    );
}

#[test]
fn neither_the_challenge_nor_the_receipt_prints_its_bytes() {
    let own = challenge(1);
    let demand = ReadinessDemand::new(&own, identity(7, 8));
    let receipt = demand.attest(&session(9));

    assert_eq!(format!("{own:?}"), "ReadinessChallenge([REDACTED])");
    assert_eq!(format!("{receipt:?}"), "ReadinessReceipt([REDACTED])");
    assert_eq!(
        ReadinessRefusal::Spent.to_string(),
        "readiness refused: Spent"
    );
    assert_eq!(demand.identity(), &identity(7, 8));
    assert_eq!(demand.identity().snapshot(), &Digest::from_bytes([7; 32]));
    assert_eq!(demand.identity().launch(), &Digest::from_bytes([8; 32]));
}
