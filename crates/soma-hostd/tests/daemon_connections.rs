//! One connection is held for as long as its client lives, so the daemon cannot serve it on the
//! accept loop.
//!
//! A machine host registers with the Host Runtime and then holds that connection for as long as
//! it owns its machine, so the first machine host to register would take the daemon away from
//! every other client behind it, including the machine hosts that start next.

#![cfg(target_os = "linux")]

mod support;

use std::{sync::mpsc, thread, time::Duration};

use soma_hostd::Request;

use support::{client, harness, limits};

/// How long a client waits for a reply before this test calls the daemon starved.
const PATIENCE: Duration = Duration::from_secs(10);

#[test]
fn a_held_connection_does_not_take_the_daemon_from_the_next_client() {
    let harness = harness(limits(2, 6));
    let directory = tempfile::tempdir().expect("a scratch directory");
    let socket = directory.path().join("allocator.sock");
    client::daemon_on(&harness.runtime, &harness.volumes, &socket);

    // The first client is answered and then holds its connection, which is what a machine host
    // does for as long as it owns its machine.
    let held = client::connect(&socket);
    held.call(&Request::Reconcile)
        .expect("the first client is answered");

    // The second one asks on its own thread, so a daemon held by the first shows up as a missing
    // answer rather than as a hung test.
    let (sender, receiver) = mpsc::channel();
    let asking = socket.clone();
    let _asker = thread::spawn(move || {
        let client = client::connect(&asking);
        let _ = sender.send(client.call(&Request::Reconcile));
    });
    let answered = receiver.recv_timeout(PATIENCE);
    assert!(
        answered.is_ok(),
        "a client that connected behind a held connection was never answered"
    );
    assert!(
        answered.expect("an answer").is_ok(),
        "the second client received a reply"
    );
    // `held` stays alive to the end of the test: the assertion above is exactly the case where a
    // connected client is not the client being served.
    drop(held);
}
