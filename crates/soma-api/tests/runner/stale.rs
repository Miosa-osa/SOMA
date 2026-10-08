//! A stale feed stops creates and nothing else.

use std::{sync::Arc, time::Duration};

use crate::{
    clients::{create_when_ready, h2},
    control_plane::start_control_plane,
    facade::{Engine, opener},
    support::{TOKEN, config, eventually, scratch, snapshot, start_runner},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stale_feed_refuses_creates_but_keeps_serving_existing_sandboxes() {
    let (control_plane, state) = start_control_plane().await;
    let engine = Arc::new(Engine::default());
    let journal = scratch("stale");
    let handle = start_runner(config(control_plane, &journal, 1), opener(&engine)).await;
    let address = handle.address;
    eventually("the feed connection", || !state.connections().is_empty()).await;
    snapshot(&state, 1);
    let id = create_when_ready(address, "").await.body["id"]
        .as_str()
        .expect("id")
        .to_owned();

    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let refused = h2(address, "POST", "/api/v1/sandboxes", Some(TOKEN), "").await;
    assert_eq!(refused.status, 503);
    assert_eq!(refused.body, serde_json::json!({"error": "feed_stale"}));
    assert_eq!(refused.headers["retry-after"], "0");

    let exec = h2(
        address,
        "POST",
        &format!("/api/v1/sandboxes/{id}/exec"),
        Some(TOKEN),
        r#"{"command":"true"}"#,
    )
    .await;
    assert_eq!(exec.status, 200, "existing sandboxes keep being served");

    let health = h2(address, "GET", "/healthz", None, "").await;
    assert!(health.body["feed_age_ms"].as_u64().expect("age") >= 1_000);
}
