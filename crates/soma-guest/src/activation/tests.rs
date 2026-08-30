use super::*;

fn scope() -> ActivationScope {
    ActivationScope::new([1; 16], [2; 16], 3, [4; 32]).expect("scope")
}

#[test]
fn scopes_reject_every_zero_field() {
    assert_eq!(
        ActivationScope::new([0; 16], [2; 16], 3, [4; 32]),
        Err(Error::InvalidActivationScope)
    );
    assert_eq!(
        ActivationScope::new([1; 16], [0; 16], 3, [4; 32]),
        Err(Error::InvalidActivationScope)
    );
    assert_eq!(
        ActivationScope::new([1; 16], [2; 16], 0, [4; 32]),
        Err(Error::InvalidActivationScope)
    );
    assert_eq!(
        ActivationScope::new([1; 16], [2; 16], 3, [0; 32]),
        Err(Error::InvalidActivationScope)
    );
}

#[test]
fn challenges_reject_zero_bytes_and_stay_out_of_diagnostics() {
    assert_eq!(
        ActivationChallenge::from_bytes([0; 32]),
        Err(Error::InvalidKeyMaterial)
    );
    let challenge = ActivationChallenge::from_bytes([9; 32]).expect("challenge");
    assert_eq!(challenge.to_bytes(), [9; 32]);
    assert_eq!(challenge, challenge.clone());
    assert_ne!(
        challenge,
        ActivationChallenge::from_bytes([8; 32]).expect("other")
    );
    assert_eq!(format!("{challenge:?}"), "ActivationChallenge([REDACTED])");
    let generated = ActivationChallenge::generate().expect("fresh challenge");
    assert_ne!(generated.to_bytes(), [0; 32]);
    assert_ne!(generated, challenge);
}

#[test]
fn receipts_round_trip_and_reject_zero_halves() {
    let challenge = ActivationChallenge::from_bytes([5; 32]).expect("challenge");
    let tag = challenge.tag(&scope(), &[6; 32]).expect("tag");
    let receipt = ActivationReceipt::new([6; 32], tag);
    let encoded = receipt.to_bytes();
    assert_eq!(encoded.len(), ActivationReceipt::LEN);
    assert_eq!(
        ActivationReceipt::from_bytes(&encoded).expect("decodes"),
        receipt
    );
    assert_eq!(receipt.transcript(), &[6; 32]);
    assert_eq!(format!("{receipt:?}"), "ActivationReceipt([REDACTED])");
    let mut zero_transcript = encoded;
    zero_transcript[..32].fill(0);
    assert_eq!(
        ActivationReceipt::from_bytes(&zero_transcript),
        Err(Error::ActivationReceiptRejected)
    );
    let mut zero_tag = encoded;
    zero_tag[32..].fill(0);
    assert_eq!(
        ActivationReceipt::from_bytes(&zero_tag),
        Err(Error::ActivationReceiptRejected)
    );
}

#[test]
fn verification_binds_the_challenge_scope_and_transcript() {
    let challenge = ActivationChallenge::from_bytes([5; 32]).expect("challenge");
    let transcript = [6; 32];
    let receipt = ActivationReceipt::new(
        transcript,
        challenge.tag(&scope(), &transcript).expect("tag"),
    );
    assert_eq!(challenge.verify(&scope(), &receipt), Ok(()));

    for other in [
        ActivationScope::new([0xaa; 16], [2; 16], 3, [4; 32]).expect("other instance"),
        ActivationScope::new([1; 16], [0xbb; 16], 3, [4; 32]).expect("other operation"),
        ActivationScope::new([1; 16], [2; 16], 4, [4; 32]).expect("other generation"),
        ActivationScope::new([1; 16], [2; 16], 3, [0xcc; 32]).expect("other intent"),
    ] {
        assert_eq!(
            challenge.verify(&other, &receipt),
            Err(Error::ActivationReceiptRejected)
        );
    }

    let other_challenge = ActivationChallenge::from_bytes([7; 32]).expect("other challenge");
    assert_eq!(
        other_challenge.verify(&scope(), &receipt),
        Err(Error::ActivationReceiptRejected)
    );

    let moved = ActivationReceipt::new([0x11; 32], receipt.tag);
    assert_eq!(
        challenge.verify(&scope(), &moved),
        Err(Error::ActivationReceiptRejected)
    );
}
