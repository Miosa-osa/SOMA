use soma_guest::{
    Error, InitiatorHandshake, InstancePsk, ResponderHandshake, ResponderKeypair, SessionBinding,
};

fn binding(generation: u8, instance: u8, operation: u8, launch_nonce: u8) -> SessionBinding {
    SessionBinding::new(
        [generation; 32],
        [instance; 16],
        [operation; 16],
        [launch_nonce; 32],
    )
    .expect("valid binding")
}

#[test]
fn responder_rejects_an_initiator_with_the_wrong_instance_psk() {
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let host_psk = InstancePsk::provision_for([2; 16], [5; 32]).expect("host PSK");
    let guest_psk = InstancePsk::provision_for([2; 16], [6; 32]).expect("guest PSK");
    let context = binding(1, 2, 3, 4);
    let (_, first) = InitiatorHandshake::start(&context, keypair.public_key(), &host_psk)
        .expect("first message");

    let result = ResponderHandshake::accept(&context, keypair.private_key(), &guest_psk, &first);
    assert_eq!(
        result.expect_err("wrong PSK must fail"),
        Error::AuthenticationFailed
    );
}

#[test]
fn handshake_setup_rejects_a_psk_provisioned_for_another_instance() {
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let psk = InstancePsk::provision_for([9; 16], [5; 32]).expect("instance PSK");
    let context = binding(1, 2, 3, 4);

    assert_eq!(
        InitiatorHandshake::start(&context, keypair.public_key(), &psk)
            .expect_err("mismatched Instance scope must fail"),
        Error::PskInstanceMismatch
    );
}

#[test]
fn responder_setup_rejects_a_psk_provisioned_for_another_instance() {
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let host_psk = InstancePsk::provision_for([2; 16], [5; 32]).expect("host PSK");
    let guest_psk = InstancePsk::provision_for([9; 16], [5; 32]).expect("guest PSK");
    let context = binding(1, 2, 3, 4);
    let (_, first) = InitiatorHandshake::start(&context, keypair.public_key(), &host_psk)
        .expect("first message");

    assert_eq!(
        ResponderHandshake::accept(&context, keypair.private_key(), &guest_psk, &first)
            .expect_err("mismatched Instance scope must fail"),
        Error::PskInstanceMismatch
    );
}

#[test]
fn responder_rejects_a_host_that_pinned_another_static_key() {
    let expected = ResponderKeypair::generate().expect("expected keypair");
    let actual = ResponderKeypair::generate().expect("actual keypair");
    let psk = InstancePsk::provision_for([2; 16], [5; 32]).expect("instance PSK");
    let context = binding(1, 2, 3, 4);
    let (_, first) =
        InitiatorHandshake::start(&context, expected.public_key(), &psk).expect("first message");

    let result = ResponderHandshake::accept(&context, actual.private_key(), &psk, &first);
    assert_eq!(
        result.expect_err("wrong key must fail"),
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

#[test]
fn a_prior_response_cannot_finish_a_fresh_launch_handshake() {
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let psk = InstancePsk::provision_for([2; 16], [5; 32]).expect("instance PSK");
    let old_context = binding(1, 2, 3, 4);
    let (old_waiting, old_first) =
        InitiatorHandshake::start(&old_context, keypair.public_key(), &psk)
            .expect("old first message");
    let old_pending =
        ResponderHandshake::accept(&old_context, keypair.private_key(), &psk, &old_first)
            .expect("old second message");
    let old_second = old_pending.response().to_vec();
    drop(old_waiting);

    let fresh_context = binding(1, 2, 3, 9);
    let (fresh_waiting, _) = InitiatorHandshake::start(&fresh_context, keypair.public_key(), &psk)
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
    let psk = InstancePsk::provision_for([2; 16], [5; 32]).expect("instance PSK");
    let context = binding(1, 2, 3, 4);
    let (old_waiting, old_first) =
        InitiatorHandshake::start(&context, keypair.public_key(), &psk).expect("old first message");
    let old_pending = ResponderHandshake::accept(&context, keypair.private_key(), &psk, &old_first)
        .expect("old second message");
    let old_second = old_pending.response().to_vec();
    drop(old_waiting);

    let (fresh_waiting, fresh_first) =
        InitiatorHandshake::start(&context, keypair.public_key(), &psk)
            .expect("fresh first message");
    assert_ne!(
        fresh_first, old_first,
        "fresh ephemeral must change message one"
    );

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
    let psk = InstancePsk::provision_for([2; 16], [5; 32]).expect("instance PSK");
    let context = binding(1, 2, 3, 4);
    let (_, first) =
        InitiatorHandshake::start(&context, keypair.public_key(), &psk).expect("first message");
    let mut truncated = first.clone();
    truncated.pop();
    let mut trailing = first;
    trailing.push(0);

    for malformed in [&truncated, &trailing] {
        let result = ResponderHandshake::accept(&context, keypair.private_key(), &psk, malformed);
        assert_eq!(
            result.expect_err("malformed message must fail"),
            Error::AuthenticationFailed
        );
    }
}

#[test]
fn initiator_rejects_truncated_or_trailing_second_message_bytes() {
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let psk = InstancePsk::provision_for([2; 16], [5; 32]).expect("instance PSK");
    let context = binding(1, 2, 3, 4);

    for trailing in [false, true] {
        let (waiting, first) =
            InitiatorHandshake::start(&context, keypair.public_key(), &psk).expect("first message");
        let pending = ResponderHandshake::accept(&context, keypair.private_key(), &psk, &first)
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

fn assert_binding_mismatch_rejected(guest_context: SessionBinding, guest_instance: u8) {
    let keypair = ResponderKeypair::generate().expect("responder keypair");
    let host_psk = InstancePsk::provision_for([2; 16], [5; 32]).expect("host PSK");
    let guest_psk = InstancePsk::provision_for([guest_instance; 16], [5; 32]).expect("guest PSK");
    let host_context = binding(1, 2, 3, 4);
    let (_, first) = InitiatorHandshake::start(&host_context, keypair.public_key(), &host_psk)
        .expect("first message");

    let result =
        ResponderHandshake::accept(&guest_context, keypair.private_key(), &guest_psk, &first);
    assert_eq!(
        result.expect_err("binding mismatch must fail"),
        Error::AuthenticationFailed
    );
}
