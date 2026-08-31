//! Reservation is exclusive and transactional, and it is not yet reachability.

use std::net::Ipv4Addr;

use soma::TransportProtocol;

use super::*;

const GUEST: Ipv4Addr = Ipv4Addr::new(10, 200, 0, 2);

fn publication(host_port: u16, protocol: TransportProtocol) -> PortPublication {
    PortPublication::new(
        HostBind::loopback_v4(),
        HostPort::from_u16(host_port),
        80,
        protocol,
    )
    .expect("publication")
}

#[test]
fn automatic_ports_resolve_and_fixed_conflicts_roll_back_the_transaction() {
    let automatic = reserve(&[publication(0, TransportProtocol::Tcp)]).expect("automatic");
    let port = automatic[0].host_port();
    assert_ne!(port, 0);
    assert_eq!(describe(&automatic[0]).3, HostPort::Automatic);
    let error = reserve(&[
        publication(0, TransportProtocol::Udp),
        publication(port, TransportProtocol::Tcp),
    ])
    .expect_err("conflict");
    assert_eq!(error, Error::PortUnavailable);
    drop(automatic);
    let again = reserve(&[publication(port, TransportProtocol::Tcp)]).expect("released");
    assert_eq!(again[0].host_port(), port);
    assert_eq!(
        attach_proxy(&again[0]).expect_err("proxy"),
        Error::Unimplemented("ingress proxy attachment")
    );
}

#[test]
fn a_reserved_tcp_port_refuses_a_connection_instead_of_accepting_one_it_cannot_serve() {
    let held = reserve(&[publication(0, TransportProtocol::Tcp)]).expect("reserved");
    let address = std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, held[0].host_port()));
    let refused =
        std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(500));
    assert!(
        refused.is_err(),
        "a reservation must not answer before activation publishes it"
    );
}

#[test]
fn publishing_a_reservation_yields_the_mapping_activation_installs() {
    let held = reserve(&[publication(0, TransportProtocol::Tcp)]).expect("reserved");
    let mapping = publish(&held[0], GUEST).expect("mapping");
    assert_eq!(mapping.host_port(), held[0].host_port());
    assert_eq!(mapping.guest_port(), 80);
    assert!(mapping.binds_loopback());
    assert_eq!(publish_all(&held, GUEST).expect("all"), vec![mapping]);
}

#[test]
fn ipv6_binds_carry_an_explicit_v6only_value_and_cannot_be_translated_yet() {
    let publication = PortPublication::new(
        HostBind::ipv6(std::net::Ipv6Addr::LOCALHOST, true).expect("bind"),
        HostPort::Automatic,
        80,
        TransportProtocol::Tcp,
    )
    .expect("publication");
    let held = reserve(&[publication]).expect("v6");
    assert_eq!(describe(&held[0]).2, Some(true));
    assert_eq!(
        publish(&held[0], GUEST).expect_err("no IPv6 translation"),
        Error::Unimplemented("ingress forwarding for an IPv6 bind")
    );
}
