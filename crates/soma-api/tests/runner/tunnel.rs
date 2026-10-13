//! `GET /api/v1/sandboxes/{id}/tunnel/{port}` on the public runner: real TLS, the HTTP/1.1
//! upgrade, a fake control plane serving the key, and a fake facade whose guest echoes.

use std::{
    net::SocketAddr,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use rustls_pki_types::ServerName;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_rustls::client::TlsStream;

use crate::{
    clients::{client_tls, create_when_ready, h2, with_headers},
    control_plane::start_control_plane,
    facade::{Engine, opener},
    support::{OTHER_TOKEN, TOKEN, document, eventually, scratch, snapshot, start_runner},
};

const BANNER: &str = "guest port 9223 ready\n";

/// A runner serving one created sandbox, with the tunnel block `tunnel` (or the defaults).
async fn runner_with_sandbox(
    name: &str,
    tunnel: Option<serde_json::Value>,
) -> (SocketAddr, String, Arc<Engine>) {
    let (control_plane, state) = start_control_plane().await;
    let engine = Arc::new(Engine::default());
    let journal = scratch(name);
    let mut document = document(control_plane, &journal, 900);
    if let Some(tunnel) = tunnel {
        document["tunnel"] = tunnel;
    }
    let config =
        soma_api::runner::RunnerConfig::parse(&serde_json::to_vec(&document).expect("json"))
            .expect("valid config");
    let handle = start_runner(config, opener(&engine)).await;
    let address = handle.address;
    // The runner must outlive the test body; the handle owns its threads.
    std::mem::forget(handle);
    eventually("the feed connection", || !state.connections().is_empty()).await;
    snapshot(&state, 1);
    // The control plane's feed state lives as long as the test needs the key.
    std::mem::forget(state);
    let created = create_when_ready(address, "").await;
    let id = created.body["id"].as_str().expect("id").to_owned();
    (address, id, engine)
}

async fn tls(address: SocketAddr, alpn: &[&[u8]]) -> TlsStream<TcpStream> {
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client_tls(alpn)));
    let tcp = TcpStream::connect(address).await.expect("connect");
    connector
        .connect(ServerName::try_from("localhost").expect("name"), tcp)
        .await
        .expect("TLS")
}

fn upgrade_request(id: &str, port: &str, token: Option<&str>) -> String {
    let authorization = token.map_or_else(String::new, |token| {
        format!("authorization: Bearer {token}\r\n")
    });
    format!(
        "GET /api/v1/sandboxes/{id}/tunnel/{port} HTTP/1.1\r\nhost: localhost\r\n{authorization}connection: Upgrade\r\nupgrade: soma-tunnel\r\n\r\n"
    )
}

/// Sends `request` on a fresh HTTP/1.1 connection and returns it with the head and what trailed.
async fn open(address: SocketAddr, request: &str) -> (TlsStream<TcpStream>, String, Vec<u8>) {
    let mut stream = tls(address, &[b"http/1.1"]).await;
    stream.write_all(request.as_bytes()).await.expect("request");
    let mut seen = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        if let Some(end) = seen.windows(4).position(|window| window == b"\r\n\r\n") {
            let rest = seen.split_off(end + 4);
            return (stream, String::from_utf8(seen).expect("ASCII head"), rest);
        }
        let read = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut chunk))
            .await
            .expect("a head in time")
            .expect("read");
        assert!(read > 0, "closed before a head: {seen:?}");
        seen.extend_from_slice(&chunk[..read]);
    }
}

async fn read_exactly(
    stream: &mut TlsStream<TcpStream>,
    mut have: Vec<u8>,
    want: usize,
) -> Vec<u8> {
    let mut chunk = [0_u8; 4096];
    while have.len() < want {
        let read = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut chunk))
            .await
            .expect("bytes in time")
            .expect("read");
        assert!(read > 0, "ended at {} of {want}", have.len());
        have.extend_from_slice(&chunk[..read]);
    }
    have
}

fn status(head: &str) -> &str {
    head.split(' ').nth(1).expect("status")
}

/// The JSON body that follows `head`, read by its `content-length` (the connection stays open).
async fn body_of(mut stream: TlsStream<TcpStream>, head: &str, have: Vec<u8>) -> serde_json::Value {
    let length = head
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(str::to_owned)
        })
        .and_then(|value| value.trim().parse::<usize>().ok())
        .expect("a content-length");
    let body = read_exactly(&mut stream, have, length).await;
    serde_json::from_slice(&body).expect("a JSON body")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_upgrade_becomes_the_guests_stream() {
    let (address, id, engine) = runner_with_sandbox("tunnel-stream", None).await;
    let (mut stream, head, rest) = open(address, &upgrade_request(&id, "9223", Some(TOKEN))).await;
    assert_eq!(status(&head), "101", "{head}");
    assert!(
        head.to_ascii_lowercase().contains("upgrade: soma-tunnel"),
        "{head}"
    );

    assert_eq!(
        read_exactly(&mut stream, rest, BANNER.len()).await,
        BANNER.as_bytes()
    );
    stream.write_all(b"hello guest").await.expect("write");
    assert_eq!(
        read_exactly(&mut stream, Vec::new(), 11).await,
        b"hello guest"
    );

    let payload: Vec<u8> = (0..300_000_u32)
        .map(|n| u8::try_from(n % 253).expect("byte"))
        .collect();
    let (mut reader, mut writer) = tokio::io::split(stream);
    let outgoing = payload.clone();
    let send = tokio::spawn(async move {
        writer.write_all(&outgoing).await.expect("payload");
        writer
    });
    let mut echoed = vec![0_u8; payload.len()];
    reader.read_exact(&mut echoed).await.expect("echo");
    assert_eq!(echoed, payload);
    let mut writer = send.await.expect("sender");

    assert_eq!(engine.tunnels.load(Ordering::SeqCst), 1);
    // The guest ends the stream and the caller sees EOF.
    writer.write_all(b"quit").await.expect("quit");
    let mut tail = Vec::new();
    reader.read_to_end(&mut tail).await.expect("EOF");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refusals_are_statuses_before_any_upgrade() {
    let (address, id, engine) = runner_with_sandbox("tunnel-refusals", None).await;

    // No key, a key nobody published.
    let (_s, head, _) = open(address, &upgrade_request(&id, "9223", None)).await;
    assert_eq!(status(&head), "401", "{head}");
    let (_s, head, _) = open(address, &upgrade_request(&id, "9223", Some(OTHER_TOKEN))).await;
    assert_eq!(status(&head), "401", "{head}");

    // A plain GET, and HTTP/2 (no Upgrade exists there), are told to upgrade properly.
    let plain = format!(
        "GET /api/v1/sandboxes/{id}/tunnel/9223 HTTP/1.1\r\nhost: localhost\r\nauthorization: Bearer {TOKEN}\r\n\r\n"
    );
    let (_s, head, _) = open(address, &plain).await;
    assert_eq!(status(&head), "426", "{head}");
    let over_h2 = with_headers(
        address,
        b"h2",
        ("GET", &format!("/api/v1/sandboxes/{id}/tunnel/9223")),
        Some(TOKEN),
        &[("connection", "upgrade"), ("upgrade", "soma-tunnel")],
        "",
    )
    .await;
    // HTTP/2 forbids the Connection header, so it may be refused before the route anyway.
    assert!(matches!(over_h2.status, 426 | 400), "{}", over_h2.status);

    // Unknown sandbox, another host's, bad ids and ports; the last character is rewritten.
    let suffix = if id.ends_with('0') { "1" } else { "0" };
    let unknown = format!("{}{suffix}", &id[..id.len() - 1]);
    let (_s, head, _) = open(address, &upgrade_request(&unknown, "9223", Some(TOKEN))).await;
    assert_eq!(status(&head), "404", "{head}");
    let elsewhere = format!("4{}", &id[1..]);
    let (_s, head, _) = open(address, &upgrade_request(&elsewhere, "9223", Some(TOKEN))).await;
    assert_eq!(status(&head), "421", "{head}");
    let (_s, head, _) = open(address, &upgrade_request("nope", "9223", Some(TOKEN))).await;
    assert_eq!(status(&head), "400", "{head}");
    for port in ["0", "70000", "http"] {
        let (_s, head, _) = open(address, &upgrade_request(&id, port, Some(TOKEN))).await;
        assert_eq!(status(&head), "400", "port {port}: {head}");
    }
    let (_s, head, _) = open(address, &upgrade_request(&id, "22", Some(TOKEN))).await;
    assert_eq!(status(&head), "403", "a port outside the policy: {head}");

    assert_eq!(
        engine.tunnels.load(Ordering::SeqCst),
        0,
        "no refusal reached the facade"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_hosts_refusals_name_themselves() {
    let (address, id, engine) = runner_with_sandbox("tunnel-host-refusals", None).await;
    for (failure, status_code, code) in [
        (
            soma_api::TunnelFailure::NoNetwork,
            "422",
            "SANDBOX_HAS_NO_NETWORK",
        ),
        (soma_api::TunnelFailure::NotFound, "404", "NOT_FOUND"),
        (
            soma_api::TunnelFailure::Unreachable,
            "502",
            "GUEST_UNREACHABLE",
        ),
        (
            soma_api::TunnelFailure::PortNotAllowed,
            "403",
            "PORT_NOT_ALLOWED",
        ),
        (
            soma_api::TunnelFailure::Unavailable,
            "503",
            "TUNNEL_UNAVAILABLE",
        ),
    ] {
        *engine.tunnel_failure.lock().expect("failure") = Some(failure);
        let (stream, head, rest) = open(address, &upgrade_request(&id, "9223", Some(TOKEN))).await;
        assert_eq!(status(&head), status_code, "{head}");
        let body = body_of(stream, &head, rest).await;
        assert_eq!(body["error"]["code"], code, "{body}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sandbox_holds_only_as_many_tunnels_as_the_config_allows() {
    let tunnel = serde_json::json!({"max_per_sandbox": 1, "ports": [[9223, 9224], [3000, 3000]]});
    let (address, id, _engine) = runner_with_sandbox("tunnel-limit", Some(tunnel)).await;
    let (first, head, _) = open(address, &upgrade_request(&id, "9223", Some(TOKEN))).await;
    assert_eq!(status(&head), "101", "{head}");
    let (_s, head, _) = open(address, &upgrade_request(&id, "3000", Some(TOKEN))).await;
    assert_eq!(status(&head), "429", "{head}");
    drop(first);
    // The pump notices the closed client and gives the place back.
    for _ in 0..100 {
        let (_s, head, _) = open(address, &upgrade_request(&id, "3000", Some(TOKEN))).await;
        if status(&head) == "101" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("a closed tunnel never gave its place back");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_idle_tunnel_is_closed_and_a_busy_one_is_not() {
    let tunnel = serde_json::json!({"idle_seconds": 3});
    let (address, id, _engine) = runner_with_sandbox("tunnel-idle", Some(tunnel)).await;
    let (mut busy, head, rest) = open(address, &upgrade_request(&id, "9223", Some(TOKEN))).await;
    assert_eq!(status(&head), "101", "{head}");
    let (mut quiet, head, quiet_rest) =
        open(address, &upgrade_request(&id, "9223", Some(TOKEN))).await;
    assert_eq!(status(&head), "101", "{head}");
    let _banner = read_exactly(&mut quiet, quiet_rest, BANNER.len()).await;
    let mut seen = read_exactly(&mut busy, rest, BANNER.len()).await;

    // Keep one tunnel moving past the idle timeout while the other says nothing.
    for _ in 0..5 {
        busy.write_all(b"x").await.expect("write");
        seen = read_exactly(&mut busy, seen, BANNER.len() + 1).await;
        seen.clear();
        seen.extend_from_slice(BANNER.as_bytes());
        tokio::time::sleep(Duration::from_millis(1_000)).await;
    }
    let mut tail = Vec::new();
    quiet
        .read_to_end(&mut tail)
        .await
        .expect("a clean end after the idle timeout");
    busy.write_all(b"y")
        .await
        .expect("the busy tunnel is still open");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_destroyed_sandbox_cannot_be_tunnelled() {
    let (address, id, _engine) = runner_with_sandbox("tunnel-destroyed", None).await;
    let destroyed = h2(
        address,
        "DELETE",
        &format!("/api/v1/sandboxes/{id}"),
        Some(TOKEN),
        "",
    )
    .await;
    assert_eq!(destroyed.status, 200, "{}", destroyed.text);
    let (_s, head, _) = open(address, &upgrade_request(&id, "9223", Some(TOKEN))).await;
    assert_eq!(status(&head), "404", "{head}");
}
