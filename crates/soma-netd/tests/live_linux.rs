//! Live Linux proofs for the network broker.
//!
//! Every test is ignored by default and fails with an explicit prerequisite message without
//! `CAP_NET_ADMIN`; `scripts/netd-live-tests.sh` runs them inside the pinned privileged
//! Ubuntu 24.04 container.

#![cfg(target_os = "linux")]

mod live;

use live::{
    checks::{
        PROBE, assert_forwarding_off, assert_launch_values, assert_policy_after_activation,
        assert_sterile, public_intent, transfer_tap,
    },
    control::{Client, broker_on, current_group},
    frames::SynOutcome,
    session::{self, Wrong, forged},
    world::{self, World},
};
use soma_netd::{
    ControlAuthority, Disposition, Error, NetNamespace, NetworkIntent, ProfileDigest, Reply,
    Request, activate, broker_owner, error_code, reconcile, release,
};

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn sterile_bundle_stays_down_until_activation_and_policy_holds_after_it() {
    live::require_privilege();
    let world = World::build();
    let state = tempfile::tempdir().expect("state dir");
    let mut broker = live::broker(state.path(), 16);
    let intent = public_intent(&broker);

    let (bundle_a, instance_a, operation_a) = live::ids(0xa1);
    let sterile = broker.prepare(bundle_a).expect("prepare a");
    assert_sterile(&sterile);
    let claim = (5, broker_owner());
    let assigning = broker.assign(sterile, instance_a, operation_a, &intent, claim);
    let mut assigned = assigning.map_err(|f| f.error).expect("assign a");
    assert_launch_values(&assigned);
    let mut guest_a = transfer_tap(&assigned);
    assert!(
        guest_a.resolve_gateway(PROBE).is_none(),
        "link must be down before activation"
    );
    assert_eq!(
        guest_a.tcp_syn(world::PUBLIC_ADDRESS, world::PUBLIC_PORT, PROBE),
        SynOutcome::Silence
    );

    let host_a = session::repaired(*instance_a.as_bytes(), *operation_a.as_bytes());
    let receipt_a = session::mint(&host_a, &assigned);
    let evidence = activate(&mut assigned, &receipt_a).expect("activate a");
    assert!(evidence.forwarding);
    assert_eq!(evidence.links_raised.len(), 3);
    assert_eq!(evidence.transcript, *receipt_a.transcript());
    assert_eq!(
        activate(&mut assigned, &receipt_a),
        Err(Error::Unauthorized("activation challenge spent")),
        "a replayed receipt must be refused"
    );
    assert!(
        guest_a.resolve_gateway(PROBE).is_some(),
        "gateway ARP after activation"
    );
    assert_policy_after_activation(&mut guest_a, &world);

    let (bundle_b, instance_b, operation_b) = live::ids(0xb2);
    let sterile_b = broker.prepare(bundle_b).expect("prepare b");
    let mut assigned_b = broker
        .assign(sterile_b, instance_b, operation_b, &intent, (6, claim.1))
        .map_err(|failure| failure.error)
        .expect("assign b");
    let host_b = session::repaired(*instance_b.as_bytes(), *operation_b.as_bytes());
    let receipt_b = session::mint(&host_b, &assigned_b);
    activate(&mut assigned_b, &receipt_b).expect("activate b");
    let mut guest_b = transfer_tap(&assigned_b);
    assert!(guest_b.resolve_gateway(PROBE).is_some());
    assert_eq!(
        guest_b.tcp_syn(world::PUBLIC_ADDRESS, world::PUBLIC_PORT, PROBE),
        SynOutcome::SynAck
    );
    let peer_b = assigned_b.launch().address().into();
    assert_eq!(
        guest_a.tcp_syn(peer_b, world::PUBLIC_PORT, PROBE),
        SynOutcome::Silence,
        "peer guest dropped"
    );
    assert!(!guest_a.ping(peer_b, PROBE), "peer guest ping dropped");
    assert!(
        !guest_a.ping(assigned_b.launch().gateway().into(), PROBE),
        "peer gateway dropped"
    );
    assert_ne!(assigned.bundle().zone(), assigned_b.bundle().zone());

    let released = release(&broker, assigned);
    assert!(released.complete && released.ledger, "{released:?}");
    let report = reconcile(&broker).expect("reconcile");
    assert_eq!(report.entries.len(), 2);
    assert!(
        report
            .entries
            .iter()
            .any(|(id, _, d)| *id == bundle_a && *d == Disposition::Released)
    );
    assert!(
        report
            .entries
            .iter()
            .any(|(id, _, d)| *id == bundle_b && *d == Disposition::Consistent)
    );
    assert_eq!(report.unowned(), 0, "{report:?}");
    let released_b = release(&broker, assigned_b);
    assert!(released_b.complete);
    assert!(
        NetNamespace::list(broker.namespace_dir())
            .expect("pins")
            .is_empty()
    );
    let report = reconcile(&broker).expect("reconcile after release");
    assert!(
        report
            .entries
            .iter()
            .all(|(_, _, d)| *d == Disposition::Released)
    );
    assert_eq!(report.unowned(), 0);
    drop(guest_a);
    drop(guest_b);
    drop(world);
}

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn hundred_way_prepare_assign_activate_release_burst() {
    live::require_privilege();
    live::burst::hundred_way();
}

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn forwarding_stays_off_for_every_unauthorized_activation() {
    live::require_privilege();
    let state = tempfile::tempdir().expect("state dir");
    let mut broker = live::broker(state.path(), 16);
    let intent = public_intent(&broker);
    let mut refused = 0_usize;

    for (seed, wrong) in [
        (0xc1_u8, Wrong::Instance),
        (0xc2, Wrong::Generation),
        (0xc3, Wrong::Intent),
    ] {
        let (bundle, instance, operation) = live::ids(seed);
        let sterile = broker.prepare(bundle).expect("prepare");
        let mut assigned = broker
            .assign(sterile, instance, operation, &intent, (5, broker_owner()))
            .map_err(|failure| failure.error)
            .expect("assign");
        let receipt = forged(&assigned, wrong);

        assert_eq!(
            activate(&mut assigned, &receipt),
            Err(Error::Unauthorized("activation receipt")),
            "{wrong:?} must not authorize activation"
        );
        assert_forwarding_off(&assigned);
        assert!(
            assigned.activation_challenge().is_none(),
            "a refused attempt still consumes the single-use challenge"
        );
        assert_eq!(
            activate(&mut assigned, &receipt),
            Err(Error::Unauthorized("activation challenge spent"))
        );
        assert_forwarding_off(&assigned);
        refused += 1;
        assert!(release(&broker, assigned).complete);
    }
    assert_eq!(refused, 3);

    let (bundle, instance, operation) = live::ids(0xc4);
    let sterile = broker.prepare(bundle).expect("prepare authorized");
    let mut assigned = broker
        .assign(sterile, instance, operation, &intent, (6, broker_owner()))
        .map_err(|failure| failure.error)
        .expect("assign authorized");
    assert_forwarding_off(&assigned);
    let host = session::repaired(*instance.as_bytes(), *operation.as_bytes());
    let receipt = session::mint(&host, &assigned);
    assert!(
        activate(&mut assigned, &receipt)
            .expect("authorized activation")
            .forwarding
    );
    assert!(release(&broker, assigned).complete);
}

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn the_control_socket_grants_each_operation_only_to_its_capability() {
    live::require_privilege();
    let unauthorized = error_code(&Error::Unauthorized("peer capability"));
    let owner = broker_owner();
    let group = current_group();
    let lifecycle = ControlAuthority::new(owner, group, &[owner], &[]).expect("lifecycle only");
    let operator = ControlAuthority::new(owner, group, &[], &[owner]).expect("reconcile only");

    let state = tempfile::tempdir().expect("state dir");
    let run = tempfile::tempdir().expect("run dir");
    broker_on(
        state.path(),
        &run.path().join("operator").join("broker.sock"),
        operator,
    );
    let client = Client::connect(&run.path().join("operator").join("broker.sock"));
    for request in lifecycle_requests() {
        client.send(&request);
        assert_eq!(
            client.reply(),
            Some(Reply::Failed(unauthorized)),
            "a reconcile-only peer must not run {request:?}"
        );
    }
    client.send(&Request::Reconcile);
    assert!(matches!(client.reply(), Some(Reply::Reconciled { .. })));

    let second = tempfile::tempdir().expect("state dir");
    broker_on(
        second.path(),
        &run.path().join("host").join("broker.sock"),
        lifecycle,
    );
    let host = Client::connect(&run.path().join("host").join("broker.sock"));
    host.send(&Request::Reconcile);
    assert_eq!(
        host.reply(),
        Some(Reply::Failed(unauthorized)),
        "a lifecycle peer must not reconcile"
    );

    let third = tempfile::tempdir().expect("state dir");
    let closed = run.path().join("closed").join("broker.sock");
    broker_on(
        third.path(),
        &closed,
        ControlAuthority::new(owner, group, &[owner.wrapping_add(1)], &[]).expect("foreign"),
    );
    assert_eq!(
        Client::connect(&closed).reply(),
        None,
        "an unadmitted peer must be closed without a reply or a descriptor"
    );
}

fn lifecycle_requests() -> Vec<Request> {
    let (bundle, instance, operation) = live::ids(0xd1);
    let intent = NetworkIntent::new(
        soma_netd::EgressClass::Denied,
        Vec::new(),
        Vec::new(),
        ProfileDigest([1; 32]),
    )
    .expect("intent");
    let generation = soma_netd::CleanupGeneration::new(1).expect("generation");
    let mut bytes = [7_u8; soma_guest::ActivationReceipt::LEN];
    bytes[0] = 1;
    vec![
        Request::Claim {
            instance,
            operation,
            vsock_cid: 5,
            intent,
        },
        Request::Activate {
            bundle,
            generation,
            receipt: soma_guest::ActivationReceipt::from_bytes(&bytes).expect("receipt"),
        },
        Request::Release { bundle, generation },
    ]
}
