use crate::{
    Error, InitiatorHandshake, InstancePsk, ResponderHandshake, ResponderKeypair, SessionBinding,
};

use super::{
    AEAD_TAG, AuthenticatedSession, INNER_HEADER, MAX_RECORD_PAYLOAD, OUTER_HEADER,
    exact_ciphertext, exact_plaintext,
};

#[test]
fn authenticated_records_reject_replay_and_reordering() {
    let (mut host, mut guest) = connected_peers();
    let first = host.seal(b"zero").expect("first record");
    let second = host.seal(b"one").expect("second record");

    assert_eq!(guest.open(&first).expect("first delivery"), b"zero");
    assert_eq!(guest.open(&first), Err(Error::PeerRecordRejected));

    let (_, mut fresh_guest) = connected_peers();
    assert_eq!(fresh_guest.open(&second), Err(Error::PeerRecordRejected));
}

#[test]
fn malformed_outer_records_fail_closed() {
    for malformed in [vec![], vec![0], vec![0, 1, 0], vec![0, 26], vec![0, 0]] {
        let (_, mut guest) = connected_peers();
        assert_eq!(guest.open(&malformed), Err(Error::PeerRecordRejected));
    }
}

#[test]
fn maximum_payload_round_trips_and_local_oversize_does_not_poison() {
    let (mut host, mut guest) = connected_peers();
    let payload = vec![0xA5; MAX_RECORD_PAYLOAD];
    let encrypted = host.seal(&payload).expect("maximum record");
    assert_eq!(guest.open(&encrypted).expect("maximum payload"), payload);

    assert_eq!(
        host.seal(&vec![0; MAX_RECORD_PAYLOAD + 1]),
        Err(Error::RecordTooLarge)
    );
    let next = host.seal(b"next").expect("session remains usable");
    assert_eq!(guest.open(&next).expect("next payload"), b"next");
}

#[test]
fn authenticated_peer_cannot_skip_the_inner_sequence() {
    let mut plaintext = vec![0_u8; INNER_HEADER];
    plaintext[..8].copy_from_slice(&1_u64.to_be_bytes());
    assert_authenticated_inner_rejected(&plaintext);
}

#[test]
fn authenticated_peer_cannot_lie_about_the_inner_length() {
    let mut plaintext = vec![0_u8; INNER_HEADER + 1];
    plaintext[8..INNER_HEADER].copy_from_slice(&2_u16.to_be_bytes());
    assert_authenticated_inner_rejected(&plaintext);
}

#[test]
fn authenticated_peer_cannot_append_trailing_inner_bytes() {
    let plaintext = vec![0_u8; INNER_HEADER + 1];
    assert_authenticated_inner_rejected(&plaintext);
}

#[test]
fn every_outer_u16_declaration_obeys_the_exact_length_rule() {
    let body = [0xA5; 32];
    for declared in 0_u16..=u16::MAX {
        let mut framed = Vec::with_capacity(OUTER_HEADER + body.len());
        framed.extend_from_slice(&declared.to_be_bytes());
        framed.extend_from_slice(&body);
        assert_eq!(
            exact_ciphertext(&framed).is_ok(),
            usize::from(declared) == body.len()
        );
    }
}

#[test]
fn outer_boundary_matrix_enforces_the_minimum_authenticated_record() {
    for body_length in 0_u16..=64 {
        let mut framed = Vec::with_capacity(OUTER_HEADER + usize::from(body_length));
        framed.extend_from_slice(&body_length.to_be_bytes());
        framed.resize(OUTER_HEADER + usize::from(body_length), 0xA5);
        assert_eq!(
            exact_ciphertext(&framed).is_ok(),
            usize::from(body_length) >= AEAD_TAG + INNER_HEADER
        );
    }
}

#[test]
fn every_inner_u16_declaration_obeys_the_exact_payload_rule() {
    const ACTUAL_PAYLOAD: usize = 7;
    let mut plaintext = vec![0_u8; INNER_HEADER + ACTUAL_PAYLOAD];
    for declared in 0_u16..=u16::MAX {
        plaintext[8..INNER_HEADER].copy_from_slice(&declared.to_be_bytes());
        assert_eq!(
            exact_plaintext(0, &plaintext).is_ok(),
            usize::from(declared) == ACTUAL_PAYLOAD
        );
    }
}

#[test]
fn send_sequence_exhaustion_does_not_advance_or_poison_the_session() {
    let (mut sender, mut receiver) = connected_peers();
    sender.next_send = u64::MAX;

    assert_eq!(sender.seal(b"rejected"), Err(Error::SessionExhausted));
    assert!(!sender.poisoned);

    sender.next_send = 0;
    let record = sender
        .seal(b"accepted")
        .expect("cipher state did not advance");
    assert_eq!(
        receiver.open(&record).expect("first transport nonce"),
        b"accepted"
    );
}

#[test]
fn receive_sequence_exhaustion_rejects_and_poisons_after_authentication() {
    let (mut sender, mut receiver) = connected_peers();
    receiver.next_receive = u64::MAX;
    let mut plaintext = Vec::with_capacity(INNER_HEADER + 7);
    plaintext.extend_from_slice(&u64::MAX.to_be_bytes());
    plaintext.extend_from_slice(&7_u16.to_be_bytes());
    plaintext.extend_from_slice(b"maximum");
    let record = seal_raw(&mut sender, &plaintext);

    assert_eq!(receiver.open(&record), Err(Error::PeerRecordRejected));
    assert_eq!(receiver.open(&record), Err(Error::SessionPoisoned));
    assert_eq!(receiver.seal(b"terminal"), Err(Error::SessionPoisoned));
}

fn assert_authenticated_inner_rejected(plaintext: &[u8]) {
    let (mut hostile, mut receiver) = connected_peers();
    let framed = seal_raw(&mut hostile, plaintext);
    assert_eq!(receiver.open(&framed), Err(Error::PeerRecordRejected));
    assert_eq!(
        receiver.seal(b"must remain poisoned"),
        Err(Error::SessionPoisoned)
    );
}

fn seal_raw(session: &mut AuthenticatedSession, plaintext: &[u8]) -> Vec<u8> {
    let mut ciphertext = vec![0_u8; plaintext.len() + AEAD_TAG];
    let written = session
        .transport
        .write_message(plaintext, &mut ciphertext)
        .expect("test peer encrypts malformed plaintext");
    ciphertext.truncate(written);
    let length = u16::try_from(written).expect("test ciphertext is bounded");
    let mut framed = Vec::with_capacity(OUTER_HEADER + written);
    framed.extend_from_slice(&length.to_be_bytes());
    framed.extend_from_slice(&ciphertext);
    framed
}

fn connected_peers() -> (AuthenticatedSession, AuthenticatedSession) {
    let binding = SessionBinding::new([1; 32], [2; 16], [3; 16], [4; 32]).expect("valid binding");
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let host_psk = InstancePsk::provision_for([2; 16], [5; 32]).expect("host PSK");
    let guest_psk = InstancePsk::provision_for([2; 16], [5; 32]).expect("guest PSK");
    let (waiting, first) =
        InitiatorHandshake::start(&binding, keypair.public_key(), host_psk).expect("first message");
    let pending = ResponderHandshake::accept(&binding, keypair.private_key(), guest_psk, &first)
        .expect("second message");
    let second = pending.response().to_vec();
    let guest = pending.finish().expect("authenticated guest");
    let host = waiting.finish(&second).expect("authenticated host");
    (host, guest)
}
