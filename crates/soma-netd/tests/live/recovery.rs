//! Recovery proofs for the broker's control protocol: a lost reply and an incomplete release.
//!
//! Both drive the real daemon over its `SOCK_SEQPACKET` control socket, so they observe the
//! exact accept, authentication, and handler path a jailed host process would.

use soma_netd::{ControlAuthority, Error, Reply, Request, broker_owner, error_code};

use super as live;
use super::{
    checks::public_intent_for,
    control::{Client, broker_on, current_group},
    session,
    world::World,
};

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn a_lost_activated_reply_is_recovered_rather_than_destroying_the_machine() {
    live::require_privilege();
    let world = World::build();
    let owner = broker_owner();
    let group = current_group();
    let state = tempfile::tempdir().expect("state dir");
    let run = tempfile::tempdir().expect("run dir");
    let socket = run.path().join("host").join("broker.sock");
    broker_on(
        state.path(),
        &socket,
        ControlAuthority::new(owner, group, &[owner], &[]).expect("lifecycle only"),
    );
    let client = Client::connect(&socket);
    let (_, instance, operation) = live::ids(0xe1);
    let intent = public_intent_for(&live::profile());

    client.send(&Request::Claim {
        instance,
        operation,
        vsock_cid: 9,
        intent: intent.clone(),
    });
    client.frame().expect("the transferred descriptor frame");
    let Some(Reply::Claimed {
        bundle,
        generation,
        activation,
        ..
    }) = client.reply()
    else {
        panic!("the claim must be answered with the assignment");
    };
    let receipt = session::repaired(*instance.as_bytes(), *operation.as_bytes())
        .network_activation(&activation, generation.get(), intent.digest().0)
        .expect("activation receipt");

    client.send(&Request::Activate {
        bundle,
        generation,
        receipt,
    });
    assert_eq!(client.reply(), Some(Reply::Activated));

    // The peer never saw that reply and replays the identical request.
    client.send(&Request::Activate {
        bundle,
        generation,
        receipt,
    });
    assert_eq!(
        client.reply(),
        Some(Reply::Activated),
        "a replayed activation must return the committed result, not destroy the Machine"
    );

    client.send(&Request::Release { bundle, generation });
    assert_eq!(
        client.reply(),
        Some(Reply::Released { complete: true }),
        "the assignment must still have been live to release"
    );

    // The record is now only in the ledger; the peer it records may still replay its release.
    client.send(&Request::Release { bundle, generation });
    assert_eq!(
        client.reply(),
        Some(Reply::Released { complete: true }),
        "the recorded owner must be able to replay an idempotent release"
    );
    drop(world);
}

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn an_incomplete_release_keeps_its_operation_identity_reserved() {
    live::require_privilege();
    let world = World::build();
    let owner = broker_owner();
    let group = current_group();
    let state = tempfile::tempdir().expect("state dir");
    let run = tempfile::tempdir().expect("run dir");
    let socket = run.path().join("host").join("broker.sock");
    broker_on(
        state.path(),
        &socket,
        ControlAuthority::new(owner, group, &[owner], &[]).expect("lifecycle only"),
    );
    let client = Client::connect(&socket);
    let (_, instance, operation) = live::ids(0xe2);
    let intent = public_intent_for(&live::profile());
    let claim = Request::Claim {
        instance,
        operation,
        vsock_cid: 11,
        intent,
    };

    client.send(&claim);
    client.frame().expect("the transferred descriptor frame");
    let Some(Reply::Claimed {
        bundle, generation, ..
    }) = client.reply()
    else {
        panic!("the claim must be answered with the assignment");
    };

    // Break the namespace pin so the teardown cannot enter it and the release is incomplete.
    live::break_namespace_pin(&state.path().join("ns").join(bundle.short_hex()));
    client.send(&Request::Release { bundle, generation });
    assert_eq!(
        client.reply(),
        Some(Reply::Released { complete: false }),
        "a teardown that could not enter the namespace must report an incomplete release"
    );

    client.send(&claim);
    assert_eq!(
        client.reply(),
        Some(Reply::Failed(error_code(&Error::NotAssigned))),
        "an operation whose release left kernel objects behind must not take a second lease"
    );

    // The first release already removed the broken pin, because a failing step no longer
    // abandons the steps after it; the retry finishes what that failure left behind.
    client.send(&Request::Release { bundle, generation });
    assert_eq!(
        client.reply(),
        Some(Reply::Released { complete: true }),
        "the retried release must finish the steps the first one could not"
    );
    client.send(&claim);
    client.frame().expect("the transferred descriptor frame");
    let Some(Reply::Claimed {
        bundle, generation, ..
    }) = client.reply()
    else {
        panic!("a completed release must free the operation identity");
    };
    client.send(&Request::Release { bundle, generation });
    assert_eq!(client.reply(), Some(Reply::Released { complete: true }));
    drop(world);
}
