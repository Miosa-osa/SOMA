use super::*;

#[test]
#[allow(clippy::single_range_in_vec_init)]
fn ports_parse_as_singles_and_ranges() {
    assert_eq!(parse_ports("9223"), Some(vec![9223..=9223]));
    assert_eq!(
        parse_ports("9223-9224, 3000 ,8000-8100"),
        Some(vec![9223..=9224, 3000..=3000, 8000..=8100])
    );
}

#[test]
fn unusable_port_lists_are_refused() {
    for bad in [
        "", ",", "0", "0-10", "10-9", "70000", "a", "1-", "-1", "1,,2",
    ] {
        assert_eq!(parse_ports(bad), None, "{bad:?}");
    }
}

#[test]
fn the_default_policy_is_the_browsers_two_ports() {
    let policy = TunnelPolicy::default();
    assert!(policy.permits(9223) && policy.permits(9224));
    assert!(!policy.permits(9222) && !policy.permits(9225));
    assert!(!policy.permits(0) && !policy.permits(22) && !policy.permits(49_983));
}

#[test]
fn a_table_refuses_a_port_outside_the_policy_before_counting_it() {
    let table = TunnelTable::new(TunnelPolicy::default());
    assert_eq!(table.admit("a", 22).unwrap_err(), Refused::PortNotAllowed);
    assert_eq!(table.open_for("a"), 0);
}

#[test]
fn a_table_holds_each_sandbox_and_the_process_to_their_limits() {
    let table = TunnelTable::new(TunnelPolicy {
        per_sandbox: 2,
        total: 3,
        ..TunnelPolicy::default()
    });
    let a1 = table.admit("a", 9223).expect("first");
    let _a2 = table.admit("a", 9224).expect("second");
    assert_eq!(table.admit("a", 9223).unwrap_err(), Refused::Limit);
    let _b1 = table
        .admit("b", 9223)
        .expect("another sandbox has its own count");
    assert_eq!(
        table.admit("c", 9223).unwrap_err(),
        Refused::Limit,
        "the process is full"
    );
    drop(a1);
    assert_eq!(table.open_for("a"), 1);
    let _c1 = table.admit("c", 9223).expect("a freed place is reusable");
}

#[test]
fn a_dropped_slot_forgets_an_empty_sandbox() {
    let table = TunnelTable::new(TunnelPolicy::default());
    drop(table.admit("a", 9223).expect("slot"));
    assert_eq!(table.open_for("a"), 0);
    assert!(table.lock().per_sandbox.is_empty());
    assert_eq!(table.lock().total, 0);
}

#[test]
fn the_upgrade_needs_both_headers() {
    assert!(asks_for_upgrade(Some("Upgrade"), Some("soma-tunnel")));
    assert!(asks_for_upgrade(
        Some("keep-alive, Upgrade"),
        Some("SOMA-TUNNEL")
    ));
    assert!(!asks_for_upgrade(Some("keep-alive"), Some("soma-tunnel")));
    assert!(!asks_for_upgrade(Some("Upgrade"), Some("websocket")));
    assert!(!asks_for_upgrade(None, Some("soma-tunnel")));
    assert!(!asks_for_upgrade(Some("Upgrade"), None));
}

#[test]
fn every_failure_has_a_status_a_code_and_a_retry_hint() {
    let cases = [
        (TunnelFailure::NotFound, 404, "machine_not_found", false),
        (
            TunnelFailure::NoNetwork,
            422,
            "sandbox_has_no_network",
            false,
        ),
        (
            TunnelFailure::PortNotAllowed,
            403,
            "port_not_allowed",
            false,
        ),
        (TunnelFailure::Unreachable, 502, "guest_unreachable", true),
        (TunnelFailure::Unavailable, 503, "tunnel_unavailable", true),
    ];
    for (failure, status, code, retryable) in cases {
        let refusal = Refusal::from(failure);
        assert_eq!(
            (refusal.status, refusal.code, refusal.retryable),
            (status, code, retryable)
        );
        assert_eq!(
            refusal.public_code,
            code.to_ascii_uppercase().replace("MACHINE_", "")
        );
    }
    assert_eq!(Refusal::from(Refused::Limit).status, 429);
}

#[test]
fn the_blocking_pump_ends_when_the_guest_ends_and_reports_every_byte() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address");
    let (guest_end, mut guest) = UnixStream::pair().expect("pair");
    let server = thread::spawn(move || {
        let (client, _) = listener.accept().expect("accept");
        pump_blocking(&client, &guest_end, Duration::from_secs(30));
    });
    let mut client = TcpStream::connect(address).expect("connect");
    client.write_all(b"hello").expect("write");
    let mut got = [0_u8; 5];
    guest.read_exact(&mut got).expect("guest reads");
    assert_eq!(&got, b"hello");
    guest.write_all(b"world").expect("guest writes");
    let mut back = [0_u8; 5];
    client.read_exact(&mut back).expect("client reads");
    assert_eq!(&back, b"world");
    drop(guest);
    let mut rest = Vec::new();
    client.read_to_end(&mut rest).expect("EOF");
    assert!(rest.is_empty());
    server.join().expect("pump ends once the guest has");
}

#[test]
fn the_blocking_pump_ends_an_idle_tunnel() {
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address");
    let (guest_end, _guest) = UnixStream::pair().expect("pair");
    let server = thread::spawn(move || {
        let (client, _) = listener.accept().expect("accept");
        let started = Instant::now();
        pump_blocking(&client, &guest_end, Duration::from_secs(2));
        started.elapsed()
    });
    let _client = TcpStream::connect(address).expect("connect");
    let took = server.join().expect("pump ends");
    assert!(
        took >= Duration::from_secs(2) && took < Duration::from_secs(6),
        "{took:?}"
    );
}
