//! The whole public lifecycle and its paperwork.

use std::{
    net::SocketAddr,
    path::Path,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use crate::{
    clients::{create_when_ready, h2, h3_request, tcp_request},
    control_plane::{ControlPlaneState, start_control_plane},
    facade::{Engine, opener},
    support::{
        OTHER_TOKEN, TENANT, TOKEN, config, eventually, hash_of, scratch, snapshot, start_runner,
    },
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_runner_serves_the_public_lifecycle_and_ships_its_paperwork() {
    let (control_plane, state) = start_control_plane().await;
    let engine = Arc::new(Engine::default());
    let journal = scratch("lifecycle");
    let handle = start_runner(config(control_plane, &journal, 900), opener(&engine)).await;
    let address = handle.address;
    let id = serve_the_lifecycle(address, &state, &engine).await;
    ship_the_paperwork(&state, &journal, &id).await;
    revoke_and_reconnect(address, &state).await;
}

/// Health, feed, create, exec, 421, 401, HTTP/3, destroy; returns the destroyed sandbox id.
async fn serve_the_lifecycle(
    address: SocketAddr,
    state: &ControlPlaneState,
    engine: &Engine,
) -> String {
    // Health needs no key and answers before the feed has delivered anything.
    let health = h2(address, "GET", "/healthz", None, "").await;
    assert_eq!(health.status, 200);
    assert_eq!(health.body["ok"], true);
    assert_eq!(health.body["tag"], "3");
    assert!(health.body["feed_age_ms"].is_u64());

    // The first feed connection resumes from nothing.
    eventually("the feed connection", || !state.connections().is_empty()).await;
    assert_eq!(state.connections(), vec![0]);
    let early = h2(address, "POST", "/api/v1/sandboxes", Some(TOKEN), "{}").await;
    assert_eq!(early.status, 401, "no key is known before the snapshot");
    assert_eq!(early.body, serde_json::json!({"error": "unauthorized"}));

    snapshot(state, 1);
    let created = create_when_ready(
        address,
        r#"{"size":"xs","wait":true,"response_format":"compact","persistent":false,"runtime_profile":"soma"}"#,
    )
    .await;
    let id = created.body["id"].as_str().expect("id").to_owned();
    assert!(id.starts_with('3'), "the id carries this host's tag: {id}");
    assert_eq!(created.body["slug"], &id[..8]);
    assert_eq!(created.body["state"], "running");
    assert_eq!(created.body["template_id"], "miosa-sandbox-soma");
    assert_eq!(created.body["cpu_count"], 1);
    assert_eq!(created.body["memory_mb"], 512);
    assert_eq!(created.body["timeout_sec"], 3_600);
    assert_eq!(created.body["runner_url"], "https://3.run-us.miosa.ai");
    assert_eq!(
        created.headers["soma-runner-url"],
        "https://3.run-us.miosa.ai"
    );
    let timing = created.headers["server-timing"].to_str().expect("header");
    assert!(
        timing.starts_with("auth;dur=")
            && timing.contains(",pool;dur=")
            && timing.contains(",exec;dur="),
        "{timing}"
    );
    assert_eq!(engine.launches.load(Ordering::SeqCst), 1);

    // Exec over HTTP/1.1 on the same runner.
    let exec = tcp_request(
        address,
        b"http/1.1",
        "POST",
        &format!("/api/v1/sandboxes/{id}/exec"),
        Some(TOKEN),
        r#"{"command":"node -v"}"#,
    )
    .await;
    assert_eq!(exec.status, 200);
    assert_eq!(
        exec.body,
        serde_json::json!({"data": {"exit_code": 0, "stderr": "", "stdout": "v22.23.2\n"}})
    );
    assert!(exec.headers.contains_key("x-miosa-soma-server-tti-us"));

    // Another host's sandbox is pointed at its runner, without touching the facade.
    let foreign = format!("a{}", &id[1..]);
    let misdirected = h2(
        address,
        "POST",
        &format!("/api/v1/sandboxes/{foreign}/exec"),
        Some(TOKEN),
        r#"{"command":"true"}"#,
    )
    .await;
    assert_eq!(misdirected.status, 421);
    assert_eq!(
        misdirected.body,
        serde_json::json!({"error": "misdirected", "runner_url": "https://a.run-us.miosa.ai"})
    );

    // An unknown key is refused from memory alone.
    let unknown = h2(
        address,
        "DELETE",
        &format!("/api/v1/sandboxes/{id}"),
        Some(OTHER_TOKEN),
        "",
    )
    .await;
    assert_eq!(unknown.status, 401);
    assert_eq!(engine.executes.load(Ordering::SeqCst), 1);
    assert_eq!(engine.destroys.load(Ordering::SeqCst), 0);

    // A create over HTTP/3.
    let over_h3 = h3_request(address, "POST", "/api/v1/sandboxes", TOKEN, "").await;
    assert_eq!(over_h3.status, 201, "{}", over_h3.body);
    assert!(over_h3.headers.contains_key("server-timing"));

    destroy_twice(address, engine, &id).await;
    id
}

/// Destroy, then destroy again: the second answers the same without a facade call, and the
/// destroyed sandbox no longer runs commands.
async fn destroy_twice(address: SocketAddr, engine: &Engine, id: &str) {
    for _ in 0..2 {
        let destroyed = h2(
            address,
            "DELETE",
            &format!("/api/v1/sandboxes/{id}"),
            Some(TOKEN),
            "",
        )
        .await;
        assert_eq!(destroyed.status, 200);
        assert_eq!(
            destroyed.body,
            serde_json::json!({"id": id, "operation_id": null, "state": "destroyed", "total_runtime_sec": null})
        );
    }
    assert_eq!(engine.destroys.load(Ordering::SeqCst), 1);
    let gone = h2(
        address,
        "POST",
        &format!("/api/v1/sandboxes/{id}/exec"),
        Some(TOKEN),
        r#"{"command":"true"}"#,
    )
    .await;
    assert_eq!(gone.status, 409);
    assert_eq!(gone.body["error"]["code"], "SANDBOX_NOT_RUNNING");
}

/// The paperwork of the lifecycle above reaches the control plane and is acknowledged.
async fn ship_the_paperwork(state: &ControlPlaneState, journal: &Path, id: &str) {
    // The paperwork reaches the control plane in offset order, batched two lines at a time.
    eventually("the journal to ship", || state.journal().len() >= 5).await;
    let shipped = state.journal();
    assert_eq!(
        shipped.keys().copied().collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    // Entries are written as each response leaves, so two requests a client sent back to back
    // may land in either order; what is fixed is which requests are paperwork.
    let mut kinds: Vec<(&str, u64)> = shipped
        .values()
        .map(|line| {
            (
                line["kind"].as_str().expect("kind"),
                line["status"].as_u64().expect("status"),
            )
        })
        .collect();
    kinds.sort_unstable();
    assert_eq!(
        kinds,
        vec![
            ("create", 201),
            ("create", 201),
            ("destroy", 200),
            ("exec", 200),
            ("exec", 409)
        ]
    );
    assert!(
        shipped
            .values()
            .all(|line| line["runner"] == "miosa-host-03"
                && line["tenant_id"] == TENANT
                && line["key_id"] == "k-1")
    );
    let exec_line = shipped
        .values()
        .find(|line| line["kind"] == "exec" && line["status"] == 200)
        .expect("exec line");
    assert_eq!(exec_line["exit_code"], 0);
    let destroy_line = shipped
        .values()
        .find(|line| line["kind"] == "destroy")
        .expect("destroy line");
    assert!(destroy_line["lifetime_ms"].is_u64());
    assert_eq!(destroy_line["sandbox_id"], id);
    assert!(
        state.posts.load(Ordering::SeqCst) >= 3,
        "batches of two lines"
    );
    // The acknowledgement is stored once the control plane's answer is back.
    eventually("the acknowledgement to be stored", || {
        acked_epochs(journal) == vec!["5".to_owned()]
    })
    .await;
    let epochs: std::collections::BTreeSet<&str> = shipped
        .values()
        .map(|line| line["boot_epoch"].as_str().expect("boot_epoch"))
        .collect();
    assert_eq!(epochs.len(), 1, "one process start is one boot epoch");
}

/// A revoke reaches the runner at once, and a dropped feed resumes where it stopped.
async fn revoke_and_reconnect(address: SocketAddr, state: &ControlPlaneState) {
    state.send(&format!(
        r#"{{"seq":5,"kind":"key_revoke","key_hash":"{}"}}"#,
        hash_of(TOKEN)
    ));
    let revoked_at = Instant::now();
    let mut status = 0;
    while revoked_at.elapsed() < Duration::from_secs(1) {
        status = h2(address, "POST", "/api/v1/sandboxes", Some(TOKEN), "")
            .await
            .status;
        if status == 401 {
            break;
        }
    }
    assert_eq!(status, 401, "a revoked key is refused within one second");

    // A dropped feed reconnects from the last applied sequence number.
    state.drop_feed();
    eventually("the feed to reconnect", || state.connections().len() == 2).await;
    assert_eq!(state.connections(), vec![0, 5]);
}

/// The stored acknowledgement of every boot epoch's journal.
fn acked_epochs(journal: &Path) -> Vec<String> {
    let mut acked: Vec<String> = std::fs::read_dir(journal)
        .expect("journal directory")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_name()?.to_str()?;
            (name.starts_with("miosa-host-03.")
                && path
                    .extension()
                    .is_some_and(|extension| extension == "acked"))
            .then(|| std::fs::read_to_string(&path).ok())
            .flatten()
        })
        .map(|contents| contents.trim().to_owned())
        .collect();
    acked.sort();
    acked
}
