//! Delivery proofs against the real daemon.
//!
//! Every fixture drives the production accept, authenticate, claim, transfer, and reply path
//! over a real control socket, so the kernel decides when delivery becomes impossible and the
//! broker's own recovery has to carry the result.

use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};

use soma::{DnsPolicy, EgressPolicy, NetworkPolicy};
use soma_netd::{ControlAuthority, NetNamespace, NetworkIntent, Reply, Request, broker_owner};

use super::{
    control::{Client, broker_on, current_group},
    ids, profile,
};

/// How long a fixture waits for the single-threaded daemon to finish one lifecycle step.
const PATIENCE: Duration = Duration::from_secs(30);
/// A ceiling on the pressure burst, so a broker that never refuses fails instead of looping.
const BURST: usize = 20_000;

fn lifecycle() -> ControlAuthority {
    let owner = broker_owner();
    ControlAuthority::new(owner, current_group(), &[owner], &[]).expect("lifecycle authority")
}

fn claim(seed: u8, vsock_cid: u32) -> Request {
    let (_, instance, operation) = ids(seed);
    Request::Claim {
        instance,
        operation,
        vsock_cid,
        intent: NetworkIntent::admit(
            &NetworkPolicy::new(EgressPolicy::PublicInternet, DnsPolicy::System, Vec::new())
                .expect("policy"),
            &profile(),
        )
        .expect("intent"),
    }
}

/// Reads frames until one decodes as a reply, so the descriptor datagram is stepped over.
fn claimed(client: &Client) -> Option<Reply> {
    while let Some(frame) = client.frame() {
        if let Ok(reply) = Reply::decode(&frame) {
            return Some(reply);
        }
    }
    None
}

/// Waits, bounded, for the pinned namespace count to settle, and returns what it settled at.
fn pins_settle_at(directory: &Path, expected: usize) -> usize {
    let until = Instant::now() + PATIENCE;
    loop {
        let pins = NetNamespace::list(directory)
            .map(|list| list.len())
            .unwrap_or_default();
        if pins == expected || Instant::now() >= until {
            return pins;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn a_claim_the_peer_cannot_receive_leaves_no_bundle_and_replays_cleanly() {
    super::require_privilege();
    let state = tempfile::tempdir().expect("state dir");
    let run = tempfile::tempdir().expect("run dir");
    let socket = run.path().join("host").join("broker.sock");
    let pins = state.path().join("ns");
    broker_on(state.path(), &socket, lifecycle());

    let lost = Client::connect(&socket);
    lost.send(&claim(0xe1, 7));
    lost.stop_reading();
    assert_eq!(
        pins_settle_at(&pins, 0),
        0,
        "a claim its peer could not receive left a bundle behind"
    );
    assert!(
        lost.frame().is_none(),
        "the peer received a frame it had refused to read"
    );
    drop(lost);

    // The peer reconnects and replays the same Instance and Launch operation.
    let client = Client::connect(&socket);
    client.send(&claim(0xe1, 7));
    let Some(Reply::Claimed {
        bundle, generation, ..
    }) = claimed(&client)
    else {
        panic!("the replayed claim was refused");
    };
    assert_eq!(pins_settle_at(&pins, 1), 1, "the replay leased twice");

    client.send(&Request::Release { bundle, generation });
    assert!(matches!(
        claimed(&client),
        Some(Reply::Released { complete: true })
    ));
    assert_eq!(pins_settle_at(&pins, 0), 0, "release left a bundle behind");
}

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn one_operation_holds_one_assignment_however_often_it_is_replayed() {
    super::require_privilege();
    let state = tempfile::tempdir().expect("state dir");
    let run = tempfile::tempdir().expect("run dir");
    let socket = run.path().join("host").join("broker.sock");
    let pins = state.path().join("ns");
    broker_on(state.path(), &socket, lifecycle());

    let client = Client::connect(&socket);
    client.send(&claim(0xe2, 9));
    let first = claimed(&client).expect("the first claim");
    let Reply::Claimed {
        bundle, generation, ..
    } = first
    else {
        panic!("the first claim was refused: {first:?}");
    };
    drop(client);

    // A peer whose reply delivery was uncertain replays the same operation on a fresh
    // connection and receives the same assignment, not a second lease.
    let replay = Client::connect(&socket);
    replay.send(&claim(0xe2, 9));
    let second = claimed(&replay).expect("the replayed claim");
    assert_eq!(second, first, "the replay produced a different assignment");
    assert_eq!(pins_settle_at(&pins, 1), 1, "the replay leased twice");

    // A replay that changes a bound field is a mismatch rather than a second lease.
    replay.send(&claim(0xe2, 10));
    assert_eq!(
        claimed(&replay),
        Some(Reply::Failed(soma_netd::error_code(
            &soma_netd::Error::ReplayMismatch
        )))
    );
    assert_eq!(pins_settle_at(&pins, 1), 1, "a mismatch leased a bundle");

    replay.send(&Request::Release { bundle, generation });
    assert!(matches!(
        claimed(&replay),
        Some(Reply::Released { complete: true })
    ));
    assert_eq!(pins_settle_at(&pins, 0), 0, "release left a bundle behind");
}

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn a_peer_that_stops_reading_its_replies_is_disconnected_rather_than_served_forever() {
    super::require_privilege();
    let state = tempfile::tempdir().expect("state dir");
    let run = tempfile::tempdir().expect("run dir");
    let socket = run.path().join("host").join("broker.sock");
    broker_on(state.path(), &socket, lifecycle());

    // Reconciliation is refused for this peer, so the burst creates no kernel object and the
    // only thing under pressure is reply delivery itself.
    let client = Client::connect(&socket);
    let mut offered = 0;
    while offered < BURST && client.try_send(&Request::Reconcile) {
        offered += 1;
    }
    assert!(
        offered < BURST,
        "the broker accepted {offered} unread replies without refusing delivery"
    );

    // The replies delivered before the peer's queue filled are still queued in it, and an
    // `AF_UNIX` peer can read them after the sender closes. So the proof that the broker let
    // this peer go is that the queue ends: draining it reaches end of file rather than blocking
    // on a broker that is still answering.
    let mut drained = 0;
    while client.frame().is_some() {
        drained += 1;
        assert!(
            drained <= offered,
            "the broker delivered more replies than it was asked for"
        );
    }
    drop(client);

    let survivor = Client::connect(&socket);
    survivor.send(&Request::Reconcile);
    assert!(
        matches!(survivor.reply(), Some(Reply::Failed(_))),
        "the broker did not survive the delivery pressure"
    );
}
