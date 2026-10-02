//! A call for another host's sandbox is forwarded to the owner over the private mTLS link.

use std::{
    net::{SocketAddr, TcpListener},
    sync::{Arc, atomic::Ordering},
};

use soma_api::runner::RunnerConfig;

use crate::{
    clients::{create_when_ready, h2},
    control_plane::start_control_plane,
    facade::{Engine, opener},
    support::{
        OTHER_TOKEN, TOKEN, document, eventually, fixtures, scratch, snapshot, start_runner,
    },
};

/// A loopback port nothing listens on yet, for a private listener configured before it binds.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("address")
        .port()
}

fn runner_config(
    control_plane: SocketAddr,
    tag: char,
    private: u16,
    peers: &[(char, u16)],
) -> RunnerConfig {
    let fixtures = fixtures();
    let mut document = document(control_plane, &scratch(&format!("forward-{tag}")), 900);
    document["runner"] = serde_json::json!(format!("runner-{tag}"));
    document["host_tag"] = serde_json::json!(tag.to_string());
    let runners: serde_json::Map<String, serde_json::Value> = peers
        .iter()
        .map(|(peer, port)| {
            (
                peer.to_string(),
                serde_json::json!({"address": format!("127.0.0.1:{port}"), "server_name": "localhost"}),
            )
        })
        .collect();
    document["peers"] = serde_json::json!({
        "listen": format!("127.0.0.1:{private}"),
        "ca": fixtures.join("ca.pem"),
        "certificate": fixtures.join("peer.pem"),
        "private_key": fixtures.join("peer-key.pem"),
        "runners": runners,
    });
    RunnerConfig::parse(&serde_json::to_vec(&document).expect("encode")).expect("valid config")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_for_another_hosts_sandbox_is_answered_by_its_owner() {
    let (control_plane, state) = start_control_plane().await;
    let (private_three, private_a, nobody) = (free_port(), free_port(), free_port());
    let three_engine = Arc::new(Engine::default());
    let a_engine = Arc::new(Engine::default());
    let three = start_runner(
        runner_config(
            control_plane,
            '3',
            private_three,
            &[('a', private_a), ('b', nobody)],
        ),
        opener(&three_engine),
    )
    .await;
    let a = start_runner(
        runner_config(control_plane, 'a', private_a, &[('3', private_three)]),
        opener(&a_engine),
    )
    .await;
    eventually("both feeds", || state.connections().len() == 2).await;
    snapshot(&state, 1);

    // Created on host `a`, then used only through host `3`.
    let created = create_when_ready(a.address, "").await;
    let id = created.body["id"].as_str().expect("id").to_owned();
    assert!(id.starts_with('a'));

    let exec = h2(
        three.address,
        "POST",
        &format!("/api/v1/sandboxes/{id}/exec"),
        Some(TOKEN),
        r#"{"command":"node -v"}"#,
    )
    .await;
    assert_eq!(exec.status, 200, "{}", exec.body);
    assert_eq!(exec.body["data"]["stdout"], "v22.23.2\n");
    assert_eq!(exec.headers["soma-runner-url"], "https://a.run-us.miosa.ai");
    assert_eq!(
        a_engine.executes.load(Ordering::SeqCst),
        1,
        "the owner ran it"
    );
    assert_eq!(three_engine.executes.load(Ordering::SeqCst), 0);

    // The owner authenticates the forwarded call itself; the pipe grants nothing.
    let unknown = h2(
        three.address,
        "DELETE",
        &format!("/api/v1/sandboxes/{id}"),
        Some(OTHER_TOKEN),
        "",
    )
    .await;
    assert_eq!(unknown.status, 401);

    // Server-sent events pass through the forward.
    let streamed = h2(
        three.address,
        "POST",
        &format!("/api/v1/sandboxes/{id}/exec/stream"),
        Some(TOKEN),
        r#"{"command":"node -v"}"#,
    )
    .await;
    assert_eq!(streamed.status, 200);
    assert!(
        streamed
            .text
            .contains("event: exit\ndata: {\"exit_code\":0}"),
        "{}",
        streamed.text
    );

    unreachable_owners_answer_misdirected(three.address, &id).await;

    let destroyed = h2(
        three.address,
        "DELETE",
        &format!("/api/v1/sandboxes/{id}"),
        Some(TOKEN),
        "",
    )
    .await;
    assert_eq!(destroyed.status, 200);
    assert_eq!(a_engine.destroys.load(Ordering::SeqCst), 1);
}

/// An owner that cannot be reached (`b`), or is not configured (`c`), answers 421 with its URL.
async fn unreachable_owners_answer_misdirected(address: SocketAddr, id: &str) {
    for tag in ['b', 'c'] {
        let foreign = format!("{tag}{}", &id[1..]);
        let misdirected = h2(
            address,
            "POST",
            &format!("/api/v1/sandboxes/{foreign}/exec"),
            Some(TOKEN),
            r#"{"command":"true"}"#,
        )
        .await;
        assert_eq!(misdirected.status, 421, "tag {tag}");
        assert_eq!(
            misdirected.body["runner_url"],
            format!("https://{tag}.run-us.miosa.ai")
        );
    }
}
