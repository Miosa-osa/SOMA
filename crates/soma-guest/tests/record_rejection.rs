use soma_guest::{
    AuthenticatedSession, Error, InitiatorHandshake, InstancePsk, MAX_RECORD_PAYLOAD,
    ResponderHandshake, ResponderKeypair, SessionBinding,
};

#[test]
fn a_record_cannot_be_replayed_in_one_session() {
    let (mut host, mut guest) = connected_peers(4);
    let first = host.seal(b"request").expect("record");
    assert_eq!(guest.open(&first).expect("first delivery"), b"request");
    assert_eq!(guest.open(&first), Err(Error::PeerRecordRejected));
    assert_eq!(guest.open(&first), Err(Error::SessionPoisoned));
}

#[test]
fn records_cannot_be_delivered_out_of_order() {
    let (mut host, mut guest) = connected_peers(4);
    let _first = host.seal(b"zero").expect("first record");
    let second = host.seal(b"one").expect("second record");

    assert_eq!(guest.open(&second), Err(Error::PeerRecordRejected));
}

#[test]
fn a_record_from_an_old_launch_cannot_cross_sessions() {
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let psk = InstancePsk::provision_for([2; 16], [5; 32]).expect("instance PSK");
    let (mut old_host, _) = connected_peers_with(4, &keypair, &psk);
    let old_record = old_host.seal(b"old request").expect("old record");
    let (_, mut fresh_guest) = connected_peers_with(9, &keypair, &psk);

    assert_eq!(
        fresh_guest.open(&old_record),
        Err(Error::PeerRecordRejected)
    );
}

#[test]
fn malformed_outer_framing_fails_closed() {
    for malformed in [vec![], vec![0], vec![0, 1, 0], vec![0, 26], vec![0, 0]] {
        let (_, mut guest) = connected_peers(4);
        assert_eq!(guest.open(&malformed), Err(Error::PeerRecordRejected));
    }
}

#[test]
fn exact_maximum_payload_round_trips_and_one_more_is_rejected_locally() {
    let (mut host, mut guest) = connected_peers(4);
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

fn connected_peers(nonce: u8) -> (AuthenticatedSession, AuthenticatedSession) {
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let psk = InstancePsk::provision_for([2; 16], [5; 32]).expect("instance PSK");
    connected_peers_with(nonce, &keypair, &psk)
}

fn connected_peers_with(
    nonce: u8,
    keypair: &ResponderKeypair,
    psk: &InstancePsk,
) -> (AuthenticatedSession, AuthenticatedSession) {
    let binding =
        SessionBinding::new([1; 32], [2; 16], [3; 16], [nonce; 32]).expect("valid binding");
    let (waiting, first) =
        InitiatorHandshake::start(&binding, keypair.public_key(), psk).expect("first message");
    let pending = ResponderHandshake::accept(&binding, keypair.private_key(), psk, &first)
        .expect("second message");
    let second = pending.response().to_vec();
    let guest = pending.finish().expect("authenticated guest");
    let host = waiting.finish(&second).expect("authenticated host");
    (host, guest)
}
