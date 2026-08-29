use super::super::{InitiatorHandshake, ResponderHandshake};
use crate::{Error, InstancePsk, ResponderKeypair, SessionBinding};

#[test]
fn a_prior_response_cannot_finish_a_fresh_launch_handshake() {
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let old_binding = binding(4);
    let (old_waiting, old_first) =
        InitiatorHandshake::start(&old_binding, keypair.public_key(), psk())
            .expect("old first message");
    let old_pending =
        ResponderHandshake::accept(&old_binding, keypair.private_key(), psk(), &old_first)
            .expect("old second message");
    let old_second = old_pending.response().to_vec();
    drop(old_waiting);

    let fresh_binding = binding(9);
    let (fresh_waiting, _) = InitiatorHandshake::start(&fresh_binding, keypair.public_key(), psk())
        .expect("fresh first message");

    assert_eq!(
        fresh_waiting
            .finish(&old_second)
            .expect_err("replay must fail"),
        Error::AuthenticationFailed
    );
}

#[test]
fn a_prior_response_cannot_finish_a_fresh_handshake_with_the_same_binding() {
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let context = binding(4);
    let (old_waiting, old_first) = InitiatorHandshake::start(&context, keypair.public_key(), psk())
        .expect("old first message");
    let old_pending =
        ResponderHandshake::accept(&context, keypair.private_key(), psk(), &old_first)
            .expect("old second message");
    let old_second = old_pending.response().to_vec();
    drop(old_waiting);

    let (fresh_waiting, fresh_first) =
        InitiatorHandshake::start(&context, keypair.public_key(), psk())
            .expect("fresh first message");
    assert_ne!(fresh_first, old_first, "fresh ephemeral must change");
    assert_eq!(
        fresh_waiting
            .finish(&old_second)
            .expect_err("response from another ephemeral must fail"),
        Error::AuthenticationFailed
    );
}

#[test]
fn responder_rejects_truncated_or_trailing_handshake_bytes() {
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let context = binding(4);
    let (_, first) =
        InitiatorHandshake::start(&context, keypair.public_key(), psk()).expect("first message");
    let mut truncated = first.clone();
    truncated.pop();
    let mut trailing = first;
    trailing.push(0);

    for malformed in [&truncated, &trailing] {
        assert_eq!(
            ResponderHandshake::accept(&context, keypair.private_key(), psk(), malformed)
                .expect_err("malformed message must fail"),
            Error::AuthenticationFailed
        );
    }
}

#[test]
fn initiator_rejects_truncated_or_trailing_second_message_bytes() {
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let context = binding(4);

    for trailing in [false, true] {
        let (waiting, first) = InitiatorHandshake::start(&context, keypair.public_key(), psk())
            .expect("first message");
        let pending = ResponderHandshake::accept(&context, keypair.private_key(), psk(), &first)
            .expect("second message");
        let mut malformed = pending.response().to_vec();
        if trailing {
            malformed.push(0);
        } else {
            malformed.pop();
        }
        assert_eq!(
            waiting
                .finish(&malformed)
                .expect_err("malformed second message must fail"),
            Error::AuthenticationFailed
        );
    }
}

fn binding(launch_nonce: u8) -> SessionBinding {
    SessionBinding::new([1; 32], [2; 16], [3; 16], [launch_nonce; 32]).expect("valid binding")
}

fn psk() -> InstancePsk {
    InstancePsk::provision_for([2; 16], [5; 32]).expect("instance PSK")
}
