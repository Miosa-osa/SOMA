use soma_guest::{
    AuthenticatedSession, Error, InitiatorHandshake, InstancePsk, ResponderHandshake,
    ResponderKeypair, SessionBinding,
};

fn binding() -> SessionBinding {
    SessionBinding::new([1; 32], [2; 16], [3; 16], [4; 32]).expect("valid binding")
}

#[test]
fn peers_exchange_authenticated_records_after_two_messages() {
    let (mut host, mut guest) = connected_peers(&binding());

    let request = host.seal(b"/usr/bin/true").expect("encrypted request");
    assert_eq!(
        guest.open(&request).expect("authenticated request"),
        b"/usr/bin/true"
    );

    let terminal = guest.seal(b"exit:0").expect("encrypted terminal");
    assert_eq!(
        host.open(&terminal).expect("authenticated terminal"),
        b"exit:0"
    );
}

#[test]
fn invalid_peer_record_poisons_both_directions() {
    let (mut host, mut guest) = connected_peers(&binding());
    let mut corrupted = host.seal(b"request").expect("encrypted request");
    let last = corrupted.last_mut().expect("authentication tag");
    *last ^= 1;

    assert_eq!(guest.open(&corrupted), Err(Error::PeerRecordRejected));
    assert_eq!(guest.open(&corrupted), Err(Error::SessionPoisoned));
    assert_eq!(guest.seal(b"terminal"), Err(Error::SessionPoisoned));
}

fn connected_peers(binding: &SessionBinding) -> (AuthenticatedSession, AuthenticatedSession) {
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let psk = InstancePsk::provision_for([2; 16], [5; 32]).expect("instance PSK");

    let (waiting, first) =
        InitiatorHandshake::start(binding, keypair.public_key(), &psk).expect("first message");
    let pending = ResponderHandshake::accept(binding, keypair.private_key(), &psk, &first)
        .expect("second message");
    let second = pending.response().to_vec();
    let guest = pending.finish().expect("authenticated guest");
    let host = waiting.finish(&second).expect("authenticated host");
    (host, guest)
}
