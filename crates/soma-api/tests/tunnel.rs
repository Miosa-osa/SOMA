//! `GET /v1/sandboxes/{id}/tunnel/{port}` on the loopback service: the real accept loop, a real
//! client socket, and a fake facade whose guest echoes. The upgrade is proved at the byte level,
//! because the point of the route is what happens to the connection after the `101`.

#[allow(
    dead_code,
    reason = "each integration test uses a subset of the shared fixtures"
)]
mod support;

use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    num::NonZeroUsize,
    sync::Arc,
    thread,
    time::Duration,
};

use soma_api::{
    ApiError, CreateAdmission, TunnelFailure, serve_with_tunnels,
    tunnel::{TunnelPolicy, TunnelTable},
};
use support::{FIXTURE_INSTANCE_ID, FakeFacade, Mode};

const UPGRADE: &str = "connection: Upgrade\r\nupgrade: soma-tunnel\r\n";

fn start(policy: TunnelPolicy, failure: Option<TunnelFailure>) -> (String, Arc<TunnelTable>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    let address = listener.local_addr().expect("bound address").to_string();
    let tunnels = TunnelTable::new(policy);
    let served = Arc::clone(&tunnels);
    thread::spawn(move || {
        let open_facade = move || {
            let facade = FakeFacade::new(Mode::Succeed);
            Ok::<_, ApiError>(match failure {
                Some(failure) => facade.refusing_tunnels(failure),
                None => facade,
            })
        };
        serve_with_tunnels(
            &listener,
            open_facade,
            &CreateAdmission::new(NonZeroUsize::MIN),
            &served,
        )
    });
    (address, tunnels)
}

fn tunnel_request(port: impl std::fmt::Display, headers: &str) -> String {
    format!(
        "GET /v1/sandboxes/{FIXTURE_INSTANCE_ID}/tunnel/{port} HTTP/1.1\r\nhost: localhost\r\n{headers}\r\n"
    )
}

/// Reads one response head, returning it and any bytes that arrived behind it.
fn read_head(stream: &mut TcpStream) -> (String, Vec<u8>) {
    let mut seen = Vec::new();
    let mut byte = [0_u8; 1024];
    loop {
        if let Some(end) = seen.windows(4).position(|window| window == b"\r\n\r\n") {
            let rest = seen.split_off(end + 4);
            return (String::from_utf8(seen).expect("ASCII head"), rest);
        }
        let read = stream.read(&mut byte).expect("a response head");
        assert!(read > 0, "the connection closed before a head: {seen:?}");
        seen.extend_from_slice(&byte[..read]);
    }
}

fn read_exactly(stream: &mut TcpStream, mut have: Vec<u8>, want: usize) -> Vec<u8> {
    let mut chunk = [0_u8; 4096];
    while have.len() < want {
        let read = stream.read(&mut chunk).expect("stream bytes");
        assert!(
            read > 0,
            "the stream ended at {} of {want} bytes",
            have.len()
        );
        have.extend_from_slice(&chunk[..read]);
    }
    have
}

fn open(address: &str, request: &str) -> (TcpStream, String, Vec<u8>) {
    let mut stream = TcpStream::connect(address).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    stream.write_all(request.as_bytes()).expect("request");
    let (head, rest) = read_head(&mut stream);
    (stream, head, rest)
}

#[test]
fn an_upgrade_becomes_the_guests_stream_in_both_directions() {
    let (address, tunnels) = start(TunnelPolicy::default(), None);
    let (mut stream, head, rest) = open(
        &address,
        &tunnel_request(9223, &format!("x-soma-tenant: acme\r\n{UPGRADE}")),
    );
    assert!(head.starts_with("HTTP/1.1 101 "), "{head}");
    assert!(
        head.to_ascii_lowercase().contains("upgrade: soma-tunnel"),
        "{head}"
    );

    // The guest speaks first, and none of it is lost behind the head.
    let banner = b"guest port 9223 ready\n";
    let got = read_exactly(&mut stream, rest, banner.len());
    assert_eq!(got, banner);

    stream.write_all(b"ping").expect("write");
    assert_eq!(read_exactly(&mut stream, Vec::new(), 4), b"ping");

    // Larger than one pump chunk, to cross buffer boundaries in both directions.
    let payload: Vec<u8> = (0..200_000_u32)
        .map(|n| u8::try_from(n % 251).expect("byte"))
        .collect();
    let mut sender = stream.try_clone().expect("clone");
    let outgoing = payload.clone();
    let writer = thread::spawn(move || sender.write_all(&outgoing).expect("write payload"));
    assert_eq!(
        read_exactly(&mut stream, Vec::new(), payload.len()),
        payload
    );
    writer.join().expect("writer");

    assert_eq!(tunnels.open_for(FIXTURE_INSTANCE_ID), 1);
    // The guest ends the stream; the caller sees EOF and the slot comes back.
    stream.write_all(b"quit").expect("quit");
    let mut tail = Vec::new();
    stream
        .read_to_end(&mut tail)
        .expect("EOF after the guest closes");
    for _ in 0..100 {
        if tunnels.open_for(FIXTURE_INSTANCE_ID) == 0 {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("the tunnel's place was never given back");
}

#[test]
fn a_half_closed_client_still_receives_the_rest_of_the_answer() {
    let (address, _tunnels) = start(TunnelPolicy::default(), None);
    let (mut stream, head, rest) = open(
        &address,
        &tunnel_request(9223, &format!("x-soma-tenant: acme\r\n{UPGRADE}")),
    );
    assert!(head.starts_with("HTTP/1.1 101 "), "{head}");
    stream.write_all(b"request").expect("write");
    stream.shutdown(Shutdown::Write).expect("half close");
    // The guest echoes what it was sent and then sees EOF; both must arrive.
    let banner = b"guest port 9223 ready\n";
    let want = banner.len() + b"request".len();
    let got = read_exactly(&mut stream, rest, want);
    assert!(got.ends_with(b"request"), "{got:?}");
}

#[test]
fn bytes_sent_straight_behind_the_request_reach_the_guest() {
    let (address, _tunnels) = start(TunnelPolicy::default(), None);
    let request = format!(
        "{}early",
        tunnel_request(9223, &format!("x-soma-tenant: acme\r\n{UPGRADE}"))
    );
    let (mut stream, head, rest) = open(&address, &request);
    assert!(head.starts_with("HTTP/1.1 101 "), "{head}");
    let banner = b"guest port 9223 ready\n";
    let got = read_exactly(&mut stream, rest, banner.len() + 5);
    assert!(got.ends_with(b"early"), "{got:?}");
}

#[test]
fn every_refusal_is_a_status_before_any_upgrade() {
    let policy = TunnelPolicy {
        ports: vec![9223..=9224, 3000..=3000],
        ..TunnelPolicy::default()
    };
    let (address, _tunnels) = start(policy, None);
    let status = |request: String| {
        let (_stream, head, _rest) = open(&address, &request);
        let code = head.split(' ').nth(1).expect("status").to_owned();
        (code, head)
    };

    let (code, head) = status(tunnel_request(9223, UPGRADE));
    assert_eq!(code, "401", "no tenant identity: {head}");

    let tenant = format!("x-soma-tenant: acme\r\n{UPGRADE}");
    let (code, head) = status(tunnel_request(9223, "x-soma-tenant: acme\r\n"));
    assert_eq!(code, "426", "a plain GET: {head}");
    let (code, _) = status(tunnel_request(22, &tenant));
    assert_eq!(code, "403", "a port outside the policy");
    let (code, head) = status(tunnel_request(3000, &tenant));
    assert_eq!(code, "101", "a configured preview port: {head}");
    let (code, _) = status(tunnel_request(0, &tenant));
    assert_eq!(code, "400", "port zero");
    let (code, _) = status(tunnel_request(70_000, &tenant));
    assert_eq!(code, "400", "a port that is not a port");
    let (code, _) = status(format!(
        "POST /v1/sandboxes/{FIXTURE_INSTANCE_ID}/tunnel/9223 HTTP/1.1\r\nhost: x\r\ncontent-length: 0\r\n{tenant}\r\n"
    ));
    assert_eq!(code, "405", "a tunnel is a GET");
}

#[test]
fn the_guests_refusals_name_themselves() {
    for (failure, status, code) in [
        (TunnelFailure::NotFound, "404", "machine_not_found"),
        (TunnelFailure::NoNetwork, "422", "sandbox_has_no_network"),
        (TunnelFailure::PortNotAllowed, "403", "port_not_allowed"),
        (TunnelFailure::Unreachable, "502", "guest_unreachable"),
        (TunnelFailure::Unavailable, "503", "tunnel_unavailable"),
    ] {
        let (address, tunnels) = start(TunnelPolicy::default(), Some(failure));
        let (mut stream, head, rest) = open(
            &address,
            &tunnel_request(9223, &format!("x-soma-tenant: acme\r\n{UPGRADE}")),
        );
        assert_eq!(head.split(' ').nth(1), Some(status), "{head}");
        let mut body = rest;
        stream.read_to_end(&mut body).expect("body");
        assert!(
            String::from_utf8_lossy(&body).contains(code),
            "{code}: {body:?}"
        );
        assert_eq!(
            tunnels.open_for(FIXTURE_INSTANCE_ID),
            0,
            "a refused tunnel holds no place"
        );
    }
}

#[test]
fn a_sandbox_holds_only_as_many_tunnels_as_the_policy_allows() {
    let policy = TunnelPolicy {
        per_sandbox: 2,
        ..TunnelPolicy::default()
    };
    let (address, tunnels) = start(policy, None);
    let request = tunnel_request(9223, &format!("x-soma-tenant: acme\r\n{UPGRADE}"));
    let (first, head, _) = open(&address, &request);
    assert!(head.starts_with("HTTP/1.1 101 "), "{head}");
    let (_second, head, _) = open(&address, &request);
    assert!(head.starts_with("HTTP/1.1 101 "), "{head}");
    let (_third, head, _) = open(&address, &request);
    assert!(head.starts_with("HTTP/1.1 429 "), "{head}");

    drop(first);
    for _ in 0..200 {
        if tunnels.open_for(FIXTURE_INSTANCE_ID) < 2 {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    let (_fourth, head, _) = open(&address, &request);
    assert!(
        head.starts_with("HTTP/1.1 101 "),
        "closing one frees a place: {head}"
    );
}

#[test]
fn an_idle_tunnel_is_closed() {
    let policy = TunnelPolicy {
        idle: Duration::from_secs(2),
        ..TunnelPolicy::default()
    };
    let (address, tunnels) = start(policy, None);
    let (mut stream, head, rest) = open(
        &address,
        &tunnel_request(9223, &format!("x-soma-tenant: acme\r\n{UPGRADE}")),
    );
    assert!(head.starts_with("HTTP/1.1 101 "), "{head}");
    let _banner = read_exactly(&mut stream, rest, b"guest port 9223 ready\n".len());
    // No more bytes either way: the service closes it after the idle timeout.
    let mut tail = Vec::new();
    stream
        .read_to_end(&mut tail)
        .expect("EOF after the idle timeout");
    assert!(tail.is_empty());
    for _ in 0..100 {
        if tunnels.open_for(FIXTURE_INSTANCE_ID) == 0 {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("an idle tunnel kept its place");
}
