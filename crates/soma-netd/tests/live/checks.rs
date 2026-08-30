//! Shared live assertions: descriptor transfer, sterile state, launch values, and the
//! post-activation policy probes.

use std::{os::fd::AsFd, time::Duration};

use soma::{DnsPolicy, EgressPolicy, NetworkPolicy};
use soma_netd::{Assigned, NetworkIntent, TransferHeader, receive_tap, send_tap, seqpacket_pair};

use super::{
    frames::{Guest, SynOutcome},
    world::{self, World},
};

/// The bounded wait every live packet probe uses.
pub const PROBE: Duration = Duration::from_millis(700);

pub fn transfer_tap(assigned: &soma_netd::Assigned) -> Guest {
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

pub fn public_intent(broker: &soma_netd::Broker) -> NetworkIntent {
    NetworkIntent::admit(
        &NetworkPolicy::new(EgressPolicy::PublicInternet, DnsPolicy::System, Vec::new())
            .expect("policy"),
        broker.profile(),
    )
    .expect("intent")
}

pub fn assert_sterile(sterile: &soma_netd::SterileBundle) {
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

pub fn assert_launch_values(assigned: &soma_netd::Assigned) {
    let launch = assigned.launch();
    assert_eq!(launch.address(), [10, 200, 0, 2]);
    assert_eq!(launch.gateway(), [10, 200, 0, 1]);
    assert_eq!(launch.prefix_length(), 30);
    assert_eq!(launch.resolver(), world::DECLARED_RESOLVER.octets());
    assert_eq!(launch.vsock_cid(), 5);
    assert_eq!(launch.generation(), 1);
    assert_eq!(launch.mac(), assigned.bundle().macs().guest);
}

pub fn assert_policy_after_activation(guest_a: &mut Guest, world: &World) {
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

/// Requires that guest traffic still cannot flow inside this assignment's namespace.
pub fn assert_forwarding_off(assigned: &Assigned) {
    assigned
        .bundle()
        .namespace()
        .within(|| {
            assert_ne!(
                std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")
                    .expect("sysctl")
                    .trim(),
                "1",
                "forwarding must stay disabled"
            );
            Ok(())
        })
        .expect("inspect namespace");
}
