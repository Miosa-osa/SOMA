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

/// The private ports this test's runners will listen on, held until they are about to bind them.
///
/// A port is free again the moment the listener that asked for it is dropped, so a test that
/// chooses a port and binds it later can lose it in between to another test process doing the
/// same thing and then fail to start its runner with `AddrInUse`: a failure of this scaffolding
/// rather than of the forwarding under test. Holding the listeners until both configurations are
/// built narrows that window to the instant before the runner binds, which is as close as a test
/// can come to reserving a port the runner binds for itself.
struct ReservedPorts {
    listeners: Vec<TcpListener>,
    ports: [u16; RESERVED],
}

/// How many ports this test needs: host `3`'s, host `a`'s, and one nothing answers for.
const RESERVED: usize = 3;

impl ReservedPorts {
    /// Reserves three loopback ports.
    fn take() -> Self {
        let listeners: Vec<TcpListener> = (0..RESERVED)
            .map(|_| TcpListener::bind("127.0.0.1:0").expect("reserve a port"))
            .collect();
        let ports = std::array::from_fn(|index| {
            listeners[index]
                .local_addr()
                .expect("reserved address")
                .port()
        });
        Self { listeners, ports }
    }

    /// Gives the runners' ports up so they can bind them, and keeps the last one, the owner
    /// nothing answers for, bound for the whole test.
    fn release(mut self) -> TcpListener {
        let unreachable = self.listeners.pop().expect("the unreachable owner's port");
        drop(self.listeners);
        unreachable
    }
}

/// Holds the unreachable owner's port and closes every connection to it at once.
///
/// A port given back to the system can be bound by any other test process, and every runner a
/// test starts presents a certificate these runners trust for that address (`localhost`, from
/// the one test CA). Given back, host `b`'s port was once taken by another test's runner, whose
/// key table did not hold this test's key, so the call for `b`'s sandbox was authenticated there
/// and came back `401` instead of `421` (SOMA#154).
fn hold_unreachable(listener: TcpListener) {
    listener.set_nonblocking(true).expect("nonblocking");
    let listener = tokio::net::TcpListener::from_std(listener).expect("an async listener");
    tokio::spawn(async move {
        while let Ok((connection, _)) = listener.accept().await {
            drop(connection);
        }
    });
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
    let reserved = ReservedPorts::take();
    let [private_three, private_a, nobody] = reserved.ports;
    let three_engine = Arc::new(Engine::default());
    let a_engine = Arc::new(Engine::default());
    let three_config = runner_config(
        control_plane,
        '3',
        private_three,
        &[('a', private_a), ('b', nobody)],
    );
    let a_config = runner_config(control_plane, 'a', private_a, &[('3', private_three)]);
    // Both configurations name both ports, so the ports are given up only here, once there is
    // nothing left to build before the runners bind.
    hold_unreachable(reserved.release());
    let three = start_runner(three_config, opener(&three_engine)).await;
    let a = start_runner(a_config, opener(&a_engine)).await;
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

/// A forward that lands on a runner whose key table no snapshot has filled yet, as host `b`'s
/// call did in SOMA#154, or as any call does while its owner is starting.
///
/// That runner cannot judge the key, so it says retry rather than `401`: an unknown key and a
/// valid key it has not heard of yet are different answers, and only the first means stop.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_forward_to_a_runner_with_no_keys_yet_is_told_to_retry_not_refused() {
    let (control_plane, state) = start_control_plane().await;
    let (silent_control_plane, silent) = start_control_plane().await;
    let reserved = ReservedPorts::take();
    let [private_three, private_other, _] = reserved.ports;
    let three_config = runner_config(control_plane, '3', private_three, &[('b', private_other)]);
    let other_config = runner_config(silent_control_plane, 'a', private_other, &[]);
    hold_unreachable(reserved.release());
    let three = start_runner(three_config, opener(&Arc::new(Engine::default()))).await;
    let _other = start_runner(other_config, opener(&Arc::new(Engine::default()))).await;
    eventually("both feeds", || {
        state.connections().len() == 1 && silent.connections().len() == 1
    })
    .await;
    snapshot(&state, 1);
    // Host `3` holds the key; the runner answering for `b` has connected to its feed and has
    // been sent nothing.
    let foreign = soma_api::runner::ids::SandboxId::mint('b');
    let path = format!("/api/v1/sandboxes/{foreign}/exec");
    // Host `3` forwards only once its own snapshot is applied; until then it answers itself.
    let mut answered = h2(three.address, "POST", &path, Some(TOKEN), "{}").await;
    for _ in 0..100 {
        if answered.headers.contains_key("soma-runner-url") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        answered = h2(three.address, "POST", &path, Some(TOKEN), "{}").await;
    }

    assert_eq!(answered.status, 503, "{}", answered.body);
    assert_eq!(answered.body, serde_json::json!({"error": "feed_stale"}));
    assert_eq!(
        answered.headers["soma-runner-url"],
        "https://b.run-us.miosa.ai"
    );
}
