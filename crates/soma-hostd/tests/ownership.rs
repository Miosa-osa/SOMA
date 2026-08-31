//! An Instance belongs to the daemon, so it outlives the client connection that created it.
//!
//! Every call here goes through the shipped [`soma_hostd::client::HostClient`], because the
//! claim being proved is that a *client* addresses a Machine by identity, not that the
//! Runtime works when a test drives it in process.

#![cfg(target_os = "linux")]

mod support;

use soma_hostd::{
    FailureCode, Page, TerminalReceipt,
    client::{ClientError, LiveInstance, Registration},
};
use support::{
    client::{connect, daemon_on},
    harness, instance, intent, launch_frame, limits,
};

/// The phase code of a worker that has been assigned to an Instance.
const ASSIGNED: u8 = 4;

#[test]
fn an_instance_outlives_the_client_that_created_it() {
    let harness = harness(limits(2, 6));
    harness.pool.replenish_blocking().expect("replenish");
    let socket = harness.dir.path().join("hostd.sock");
    daemon_on(&harness.runtime, &socket);

    let creator = connect(&socket);
    let Registration::Live {
        worker,
        lease_generation,
        ..
    } = creator
        .launch(&launch_frame(&intent(11)))
        .expect("the Launch is accepted through the socket")
    else {
        panic!("a first Launch of a fresh operation is live");
    };
    drop(creator);

    // Nothing of the Instance belonged to that connection, so a second client addresses the
    // same Machine by identity alone, without ever naming a worker or a host resource.
    let observer = connect(&socket);
    assert_eq!(
        observer
            .get(instance(11))
            .expect("the Instance is still live"),
        LiveInstance {
            worker,
            lease_generation,
            phase: ASSIGNED,
        },
        "the Instance is still owned and still assigned after its creator is gone"
    );
    assert_eq!(
        observer.list(None).expect("listing"),
        Page {
            instances: vec![instance(11)],
            more: false,
        },
        "the listing reports exactly the one live Instance"
    );
    assert_eq!(
        observer.get(instance(12)),
        Err(ClientError::Refused(FailureCode::Unknown)),
        "an identity the Host never launched is refused by name rather than answered"
    );
}

#[test]
fn a_destroy_from_a_second_client_is_terminal_and_idempotent() {
    let harness = harness(limits(2, 6));
    harness.pool.replenish_blocking().expect("replenish");
    let socket = harness.dir.path().join("hostd.sock");
    daemon_on(&harness.runtime, &socket);

    let creator = connect(&socket);
    let frame = launch_frame(&intent(13));
    let Registration::Live { worker, .. } = creator.launch(&frame).expect("launch") else {
        panic!("a first Launch of a fresh operation is live");
    };
    assert_eq!(
        creator.launch(&frame).expect("replay"),
        creator.launch(&frame).expect("replay"),
        "a retry of the same operation repeats one reply rather than launching again"
    );
    drop(creator);

    let destroyer = connect(&socket);
    let receipt = TerminalReceipt {
        instance: instance(13),
        worker,
        complete: true,
    };
    assert_eq!(
        destroyer.destroy(instance(13)).expect("destroy"),
        receipt,
        "a client that never launched the Instance may still end it"
    );
    assert_eq!(
        destroyer.destroy(instance(13)).expect("repeat"),
        receipt,
        "the repeat is answered from the durable record with the same receipt"
    );
    assert_eq!(
        destroyer.list(None).expect("listing"),
        Page {
            instances: Vec::new(),
            more: false,
        },
        "the destroyed Instance is no longer live"
    );
}
