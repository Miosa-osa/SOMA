//! An Instance belongs to the daemon, so it outlives the client connection that created it.

#![cfg(target_os = "linux")]

mod support;

use soma_hostd::{FailureCode, Reply, Request, failure_code};
use support::{
    client::{Client, daemon_on},
    harness, instance, launch_request, limits,
};

#[test]
fn an_instance_outlives_the_client_that_created_it() {
    let harness = harness(limits(2, 6));
    harness.pool.replenish_blocking().expect("replenish");
    let socket = harness.dir.path().join("hostd.sock");
    daemon_on(&harness.runtime, &socket);

    let creator = Client::connect(&socket);
    let Reply::Launched {
        worker,
        lease_generation,
        ..
    } = creator.call(&launch_request(11))
    else {
        panic!("the Launch is accepted through the socket");
    };
    drop(creator);

    // Nothing of the Instance belonged to that connection, so a second client addresses the
    // same Machine by identity alone, without ever naming a worker or a host resource.
    let observer = Client::connect(&socket);
    assert_eq!(
        observer.call(&Request::Get {
            instance: instance(11)
        }),
        Reply::Live {
            worker,
            lease_generation,
            phase: 4,
        },
        "the Instance is still owned and still assigned after its creator is gone"
    );
    assert_eq!(
        observer.call(&Request::List { after: None }),
        Reply::Listed {
            instances: vec![instance(11)],
            more: false,
        },
        "the listing reports exactly the one live Instance"
    );
    assert_eq!(
        observer.call(&Request::Get {
            instance: instance(12)
        }),
        Reply::Failed(failure_code(FailureCode::Unknown)),
        "an identity the Host never launched is unknown"
    );
}

#[test]
fn a_destroy_from_a_second_client_is_terminal_and_idempotent() {
    let harness = harness(limits(2, 6));
    harness.pool.replenish_blocking().expect("replenish");
    let socket = harness.dir.path().join("hostd.sock");
    daemon_on(&harness.runtime, &socket);

    let creator = Client::connect(&socket);
    let Reply::Launched { worker, .. } = creator.call(&launch_request(13)) else {
        panic!("the Launch is accepted through the socket");
    };
    assert_eq!(
        creator.call(&launch_request(13)),
        creator.call(&launch_request(13)),
        "a retry of the same operation repeats one reply rather than launching again"
    );
    drop(creator);

    let destroyer = Client::connect(&socket);
    let receipt = Reply::Destroyed {
        worker,
        complete: true,
    };
    assert_eq!(
        destroyer.call(&Request::Destroy {
            instance: instance(13)
        }),
        receipt
    );
    assert_eq!(
        destroyer.call(&Request::Destroy {
            instance: instance(13)
        }),
        receipt,
        "the repeat is answered from the durable record with the same receipt"
    );
    assert_eq!(
        destroyer.call(&Request::List { after: None }),
        Reply::Listed {
            instances: Vec::new(),
            more: false,
        },
        "the destroyed Instance is no longer live"
    );
}
