use super::super::{InitiatorHandshake, ResponderHandshake};
use crate::{Error, InstancePsk, ResponderKeypair, SessionBinding};

#[test]
fn responder_rejects_an_initiator_with_the_wrong_instance_psk() {
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let context = binding(1, 2, 3, 4);
    let (_, first) = InitiatorHandshake::start(&context, keypair.public_key(), psk(2, 5))
        .expect("first message");

    let result = ResponderHandshake::accept(&context, keypair.private_key(), psk(2, 6), &first);
    assert_eq!(
        result.expect_err("wrong PSK must fail"),
        Error::AuthenticationFailed
    );
}

#[test]
fn initiator_rejects_a_psk_scoped_to_another_instance() {
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let context = binding(1, 2, 3, 4);

    assert_eq!(
        InitiatorHandshake::start(&context, keypair.public_key(), psk(9, 5))
            .expect_err("mismatched Instance scope must fail"),
        Error::PskInstanceMismatch
    );
}

#[test]
fn responder_rejects_a_psk_scoped_to_another_instance() {
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let context = binding(1, 2, 3, 4);
    let (_, first) = InitiatorHandshake::start(&context, keypair.public_key(), psk(2, 5))
        .expect("first message");

    assert_eq!(
        ResponderHandshake::accept(&context, keypair.private_key(), psk(9, 5), &first)
            .expect_err("mismatched Instance scope must fail"),
        Error::PskInstanceMismatch
    );
}

#[test]
fn responder_rejects_a_host_that_pinned_another_static_key() {
    let expected = ResponderKeypair::generate().expect("expected keypair");
    let actual = ResponderKeypair::generate().expect("actual keypair");
    let context = binding(1, 2, 3, 4);
    let (_, first) = InitiatorHandshake::start(&context, expected.public_key(), psk(2, 5))
        .expect("first message");

    assert_eq!(
        ResponderHandshake::accept(&context, actual.private_key(), psk(2, 5), &first)
            .expect_err("wrong key must fail"),
        Error::AuthenticationFailed
    );
}

#[test]
fn transcript_rejects_a_changed_generation() {
    assert_binding_mismatch_rejected(binding(9, 2, 3, 4), 2);
}

#[test]
fn transcript_rejects_a_changed_instance() {
    assert_binding_mismatch_rejected(binding(1, 9, 3, 4), 9);
}

#[test]
fn transcript_rejects_a_changed_operation() {
    assert_binding_mismatch_rejected(binding(1, 2, 9, 4), 2);
}

#[test]
fn transcript_rejects_a_changed_launch_nonce() {
    assert_binding_mismatch_rejected(binding(1, 2, 3, 9), 2);
}

fn assert_binding_mismatch_rejected(guest_binding: SessionBinding, guest_instance: u8) {
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let host_binding = binding(1, 2, 3, 4);
    let (_, first) = InitiatorHandshake::start(&host_binding, keypair.public_key(), psk(2, 5))
        .expect("first message");

    assert_eq!(
        ResponderHandshake::accept(
            &guest_binding,
            keypair.private_key(),
            psk(guest_instance, 5),
            &first,
        )
        .expect_err("binding mismatch must fail"),
        Error::AuthenticationFailed
    );
}

fn binding(generation: u8, instance: u8, operation: u8, launch_nonce: u8) -> SessionBinding {
    SessionBinding::new(
        [generation; 32],
        [instance; 16],
        [operation; 16],
        [launch_nonce; 32],
    )
    .expect("valid binding")
}

fn psk(instance: u8, secret: u8) -> InstancePsk {
    InstancePsk::provision_for([instance; 16], [secret; 32]).expect("instance PSK")
}
