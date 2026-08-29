//! Live Linux proofs for the network broker.
//!
//! Every test is ignored by default and fails with an explicit prerequisite message without
//! `CAP_NET_ADMIN`; `scripts/netd-live-tests.sh` runs them inside the pinned privileged
//! Ubuntu 24.04 container.

#![cfg(target_os = "linux")]

mod live;

use std::{os::fd::AsFd, time::Duration};

use live::{
    frames::{Guest, SynOutcome},
    world::{self, World},
};
use soma::{DnsPolicy, EgressPolicy, NetworkPolicy};
use soma_netd::{
    Disposition, NetNamespace, NetworkIntent, RepairAttestation, TransferHeader, activate,
    receive_tap, reconcile, release, send_tap, seqpacket_pair,
};

const PROBE: Duration = Duration::from_millis(700);

fn transfer_tap(assigned: &soma_netd::Assigned) -> Guest {
    let (broker_end, vmm_end) = seqpacket_pair().expect("seqpacket pair");
    let header = TransferHeader {
        bundle: assigned.record().bundle,
        generation: assigned.record().generation,
        intent: assigned.record().intent_digest,
    };
    send_tap(broker_end.as_fd(), &header, assigned.bundle().tap().as_fd()).expect("send tap");
    let (received, tap) = receive_tap(vmm_end.as_fd()).expect("receive tap");
    assert_eq!(received, header);
    Guest::new(tap, &assigned.launch())
}

fn public_intent(broker: &soma_netd::Broker) -> NetworkIntent {
    NetworkIntent::admit(
        &NetworkPolicy::new(EgressPolicy::PublicInternet, DnsPolicy::System, Vec::new())
            .expect("policy"),
        broker.profile(),
    )
    .expect("intent")
}

fn assert_sterile(sterile: &soma_netd::SterileBundle) {
    let names = sterile.names().clone();
    sterile
        .namespace()
        .within(|| {
            assert!(
                !std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")
                    .expect("sysctl")
                    .trim()
                    .eq("1")
            );
            let flags = std::fs::read_to_string(format!("/sys/class/net/{}/flags", names.tap)).ok();
            assert!(flags.is_none() || !flags.expect("flags").contains("0x1003"));
            Ok(())
        })
        .expect("inspect sterile namespace");
}

fn assert_launch_values(assigned: &soma_netd::Assigned) {
    let launch = assigned.launch();
    assert_eq!(launch.address(), [10, 200, 0, 2]);
    assert_eq!(launch.gateway(), [10, 200, 0, 1]);
    assert_eq!(launch.prefix_length(), 30);
    assert_eq!(launch.resolver(), world::DECLARED_RESOLVER.octets());
    assert_eq!(launch.vsock_cid(), 5);
    assert_eq!(launch.generation(), 1);
    assert_eq!(launch.mac(), assigned.bundle().macs().guest);
}

fn assert_policy_after_activation(guest_a: &mut Guest, world: &World) {
    assert!(
        guest_a.ping([10, 200, 0, 1].into(), PROBE),
        "gateway ping after activation"
    );
    assert_eq!(
        guest_a.tcp_syn(world::PUBLIC_ADDRESS, world::PUBLIC_PORT, PROBE),
        SynOutcome::SynAck,
        "public TCP egress after activation"
    );
    assert!(
        world
            .accepted_peers()
            .iter()
            .any(|peer| peer.ip() == std::net::IpAddr::V4(world::HOST_ADDRESS)),
        "world must see the masqueraded host address"
    );
    assert_eq!(
        guest_a.tcp_syn(world::METADATA, 80, PROBE),
        SynOutcome::Silence,
        "metadata endpoint must be dropped in PublicInternet mode"
    );
    assert!(
        std::net::TcpStream::connect_timeout(&(world::METADATA, 80).into(), PROBE).is_ok(),
        "the host itself can reach the metadata stand-in, so the guest drop is policy"
    );
    assert_eq!(
        guest_a
            .udp_probe(world::DECLARED_RESOLVER, 53, PROBE)
            .as_deref(),
        Some(&b"dns-ok"[..])
    );
    assert!(
        guest_a
            .udp_probe(world::UNDECLARED_RESOLVER, 53, PROBE)
            .is_none(),
        "undeclared DNS dropped"
    );
    assert!(
        !guest_a.ping(world::HOST_ADDRESS, PROBE),
        "host address must be protected"
    );
}

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
    let mut assigned = broker
        .assign(sterile, instance_a, operation_a, &intent, 5)
        .map_err(|failure| failure.error)
        .expect("assign a");
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

    let evidence =
        activate(&mut assigned, RepairAttestation::authenticated(instance_a)).expect("activate a");
    assert!(evidence.forwarding);
    assert_eq!(evidence.links_raised.len(), 3);
    assert!(
        guest_a.resolve_gateway(PROBE).is_some(),
        "gateway ARP after activation"
    );
    assert_policy_after_activation(&mut guest_a, &world);

    let (bundle_b, instance_b, operation_b) = live::ids(0xb2);
    let sterile_b = broker.prepare(bundle_b).expect("prepare b");
    let mut assigned_b = broker
        .assign(sterile_b, instance_b, operation_b, &intent, 6)
        .map_err(|failure| failure.error)
        .expect("assign b");
    activate(
        &mut assigned_b,
        RepairAttestation::authenticated(instance_b),
    )
    .expect("activate b");
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

    let released = release(&broker, assigned).expect("release a");
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
    let released_b = release(&broker, assigned_b).expect("release b");
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
    use std::time::Instant;
    live::require_privilege();
    let state = tempfile::tempdir().expect("state dir");
    let mut broker = live::broker(state.path(), 128);
    let intent = NetworkIntent::admit(
        &NetworkPolicy::new(EgressPolicy::PublicInternet, DnsPolicy::System, Vec::new())
            .expect("policy"),
        &broker.profile().clone(),
    )
    .expect("intent");
    let mut samples: [Vec<u128>; 4] = Default::default();
    for index in 0..100_u8 {
        let mut bytes = [0; 16];
        bytes[0] = 0xc0;
        bytes[1] = index;
        bytes[15] = 1;
        let bundle = soma_netd::BundleId::new(bytes).expect("bundle");
        let instance = soma_netd::InstanceId::new(bytes).expect("instance");
        let operation = soma_netd::OperationId::new(bytes).expect("operation");
        let start = Instant::now();
        let sterile = broker.prepare(bundle).expect("prepare");
        samples[0].push(start.elapsed().as_nanos());
        let start = Instant::now();
        let mut assigned = broker
            .assign(sterile, instance, operation, &intent, 3 + u32::from(index))
            .map_err(|failure| failure.error)
            .expect("assign");
        samples[1].push(start.elapsed().as_nanos());
        let start = Instant::now();
        activate(&mut assigned, RepairAttestation::authenticated(instance)).expect("activate");
        samples[2].push(start.elapsed().as_nanos());
        let start = Instant::now();
        let evidence = release(&broker, assigned).expect("release");
        samples[3].push(start.elapsed().as_nanos());
        assert!(evidence.complete, "bundle {index} incomplete: {evidence:?}");
    }
    for (name, values) in ["prepare", "assign", "activate", "release"]
        .iter()
        .zip(samples.iter_mut())
    {
        values.sort_unstable();
        let p50 = values[values.len() / 2];
        let p99 = values[values.len() * 99 / 100];
        println!(
            "burst op={name} n={} min_ns={} p50_ns={p50} p99_ns={p99} max_ns={}",
            values.len(),
            values[0],
            values[values.len() - 1]
        );
        println!("burst raw op={name} ns={values:?}");
    }
    let report = reconcile(&broker).expect("reconcile");
    assert_eq!(report.entries.len(), 100);
    assert!(
        report
            .entries
            .iter()
            .all(|(_, _, d)| *d == Disposition::Released)
    );
    assert_eq!(report.unowned(), 0);
    assert!(
        NetNamespace::list(broker.namespace_dir())
            .expect("pins")
            .is_empty()
    );
}
