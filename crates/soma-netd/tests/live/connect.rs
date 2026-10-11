//! The live proof that the broker dials a guest only through its own bundle, on an allowed port,
//! after activation.
//!
//! Two bundles are assigned. Each has a guest stand-in on its own TAP that answers a SYN on the
//! service port and on nothing else, so what reaches which guest is observable at the frame
//! level and a connection that the sandbox ruleset would drop never completes.

use std::{
    net::{IpAddr, TcpStream},
    thread,
    time::{Duration, Instant},
};

use soma_netd::{ConnectPorts, Error, NetworkIntent, Step, activate, broker_owner, release};

use super::{
    checks::{public_intent, transfer_tap},
    frames::Guest,
    session,
};

/// The guest port the allowlist and the stand-in share.
const SERVICE: u16 = 8080;
/// The bound wait for a connection that is expected to complete.
const CONNECT: Duration = Duration::from_secs(5);
/// How long a stand-in listens for a SYN that must not arrive.
const QUIET: Duration = Duration::from_millis(900);

struct Instance {
    assigned: soma_netd::Assigned,
    guest: Guest,
}

fn bring_up(
    broker: &mut soma_netd::Broker,
    intent: &NetworkIntent,
    seed: u8,
    activated: bool,
) -> Instance {
    let (bundle, instance, operation) = super::ids(seed);
    let sterile = broker.prepare(bundle).expect("prepare");
    let mut assigned = broker
        .assign(sterile, instance, operation, intent, (7, broker_owner()))
        .map_err(|failure| failure.error)
        .expect("assign");
    let mut guest = transfer_tap(&assigned);
    if activated {
        let host = session::repaired(*instance.as_bytes(), *operation.as_bytes());
        let receipt = session::mint(&host, &assigned);
        activate(&mut assigned, &receipt).expect("activate");
        assert!(guest.resolve_gateway(CONNECT).is_some(), "gateway ARP");
    }
    Instance { assigned, guest }
}

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
#[allow(clippy::single_range_in_vec_init)]
fn the_broker_dials_only_its_own_guest_on_an_allowed_port_after_activation() {
    super::require_privilege();
    let state = tempfile::tempdir().expect("state dir");
    let mut broker = super::broker(state.path(), 16)
        .with_connect_ports(ConnectPorts::new(&[SERVICE..=SERVICE]).expect("ports"));
    let intent = public_intent(&broker);

    // A bundle that is assigned but not activated has a guest nobody has repaired.
    let mut early = bring_up(&mut broker, &intent, 0xc3, false);
    assert!(
        matches!(
            broker.connect_guest(&early.assigned, SERVICE),
            Err(Error::InvalidState(_))
        ),
        "an unactivated bundle must not be dialed"
    );
    assert_eq!(early.guest.accept_syn(SERVICE, QUIET), None);
    drop(early.guest);
    assert!(release(&broker, early.assigned).complete);

    let mut first = bring_up(&mut broker, &intent, 0xc1, true);
    let mut second = bring_up(&mut broker, &intent, 0xc2, true);
    let gateway = first.assigned.bundle().leases().guest.host();
    let guest_address = first.assigned.bundle().leases().guest.guest();

    // A port off the allowlist is refused before any packet is sent.
    assert_eq!(
        broker.connect_guest(&first.assigned, SERVICE + 1).err(),
        Some(Error::Unauthorized("connect port"))
    );
    assert_eq!(first.guest.accept_syn(SERVICE + 1, QUIET), None);

    // An allowed port reaches the guest of the bundle named, from the namespace's own address,
    // and the handshake completes, which needs the ruleset to admit both directions.
    let socket = thread::scope(|scope| {
        let dial = scope.spawn(|| broker.connect_guest(&first.assigned, SERVICE));
        let source = first.guest.accept_syn(SERVICE, CONNECT);
        assert_eq!(source, Some(gateway), "the SYN must come from the gateway");
        dial.join().expect("dial thread")
    })
    .expect("the connection to the guest completes");
    let stream = TcpStream::from(socket);
    assert_eq!(
        stream.peer_addr().expect("peer").ip(),
        IpAddr::V4(guest_address)
    );
    assert_eq!(
        stream.local_addr().expect("local").ip(),
        IpAddr::V4(gateway)
    );

    // The other bundle's guest never saw it.
    assert_eq!(
        second.guest.accept_syn(SERVICE, QUIET),
        None,
        "a connection to one bundle must not reach another bundle's guest"
    );

    // A guest that never answers costs the bounded deadline and not a hang.
    let started = Instant::now();
    let silent = broker.connect_guest(&second.assigned, SERVICE);
    assert!(
        matches!(
            silent,
            Err(Error::Kernel {
                step: Step::Connect,
                ..
            })
        ),
        "{silent:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(6));

    drop(stream);
    for Instance { assigned, guest } in [first, second] {
        drop(guest);
        assert!(release(&broker, assigned).complete);
    }
}

/// A plan made for one bundle cannot reach its successor, even when the successor reuses the
/// same lease index and so the same guest address.
#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
#[allow(clippy::single_range_in_vec_init)]
fn a_plan_outliving_its_bundle_never_reaches_the_bundle_that_reuses_its_address() {
    super::require_privilege();
    let state = tempfile::tempdir().expect("state dir");
    let mut broker = super::broker(state.path(), 16)
        .with_connect_ports(ConnectPorts::new(&[SERVICE..=SERVICE]).expect("ports"));
    let intent = public_intent(&broker);

    let first = bring_up(&mut broker, &intent, 0xd1, true);
    let plan = broker
        .connect_plan(&first.assigned, SERVICE)
        .expect("a plan for an activated bundle");
    let (index, zone) = (
        first.assigned.bundle().leases().guest.index(),
        first.assigned.bundle().zone(),
    );
    let address = first.assigned.bundle().leases().guest.guest();
    drop(first.guest);
    let released = release(&broker, first.assigned);
    assert!(released.complete);
    broker.recycle(index, zone);

    let mut second = bring_up(&mut broker, &intent, 0xd2, true);
    assert_eq!(
        second.assigned.bundle().leases().guest.guest(),
        address,
        "the successor must reuse the address for this proof to mean anything"
    );
    assert!(plan.dial().is_err(), "a stale plan must not connect");
    assert_eq!(
        second.guest.accept_syn(SERVICE, QUIET),
        None,
        "the successor's guest must never see the stale plan's SYN"
    );
    drop(second.guest);
    assert!(release(&broker, second.assigned).complete);
}
