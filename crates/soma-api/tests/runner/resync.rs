//! A revoke the runner cannot apply fails closed: full resync, creates refused until it ends.

use std::sync::Arc;

use crate::{
    clients::{create_when_ready, h2},
    control_plane::start_control_plane,
    facade::{Engine, opener},
    support::{TOKEN, config, eventually, scratch, snapshot, start_runner},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_malformed_revoke_resyncs_from_zero_and_refuses_creates_until_the_snapshot_ends() {
    let (control_plane, state) = start_control_plane().await;
    let engine = Arc::new(Engine::default());
    let handle = start_runner(
        config(control_plane, &scratch("resync"), 900),
        opener(&engine),
    )
    .await;
    let address = handle.address;
    eventually("the feed connection", || state.connections() == vec![0]).await;
    snapshot(&state, 1);
    let id = create_when_ready(address, "").await.body["id"]
        .as_str()
        .expect("id")
        .to_owned();

    state.send(r#"{"seq":5,"kind":"key_revoke","key_hash":"NOT-HEX"}"#);
    eventually("a reconnect asking for everything", || {
        state.connections() == vec![0, 0]
    })
    .await;

    let refused = h2(address, "POST", "/api/v1/sandboxes", Some(TOKEN), "").await;
    assert_eq!(refused.status, 503);
    assert_eq!(refused.body, serde_json::json!({"error": "feed_stale"}));
    let exec = h2(
        address,
        "POST",
        &format!("/api/v1/sandboxes/{id}/exec"),
        Some(TOKEN),
        r#"{"command":"true"}"#,
    )
    .await;
    assert_eq!(exec.status, 200, "existing sandboxes keep working");

    snapshot(&state, 100);
    let created = create_when_ready(address, "").await;
    assert_eq!(
        created.status, 201,
        "creates resume once the snapshot has ended"
    );
}
