//! `GET` and `PATCH /api/v1/sandboxes/{id}/resources` on the public runner: key and tenant rules
//! as for every per-sandbox route, the body and status of the loopback handler behind them.

use std::sync::Arc;

use soma_api::ResizeRequest;
use soma_local::{CpuResize, DimensionState, MemoryResize, ResizeOutcome};

use crate::{
    clients::{create_when_ready, h2, with_headers},
    control_plane::start_control_plane,
    facade::{Engine, opener},
    support::{OTHER_TOKEN, TOKEN, config, eventually, scratch, snapshot, start_runner},
};

fn outcome(applied: bool) -> ResizeOutcome {
    let state = if applied {
        DimensionState::AppliedLive
    } else {
        DimensionState::Unsupported
    };
    ResizeOutcome {
        cpu: CpuResize {
            state,
            requested: Some(2),
            applied: 2,
            booted: 4,
            reason: None,
        },
        memory: MemoryResize {
            state,
            requested_mb: Some(4_096),
            plugged_mb: 2_048,
            total_mb: 4_096,
            booted_mb: 2_048,
            ceiling_mb: 8_192,
            reason: (!applied).then(|| "no virtio-mem in this Generation".to_owned()),
        },
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_runner_resizes_and_reports_a_sandbox_it_owns() {
    let (control_plane, state) = start_control_plane().await;
    let engine = Arc::new(Engine::default());
    let journal = scratch("resources");
    let handle = start_runner(config(control_plane, &journal, 900), opener(&engine)).await;
    let address = handle.address;
    eventually("the feed connection", || !state.connections().is_empty()).await;
    snapshot(&state, 1);
    let id = create_when_ready(address, "").await.body["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let path = format!("/api/v1/sandboxes/{id}/resources");

    *engine.resize_outcome.lock().expect("outcome") = Some(outcome(true));
    let changed = h2(
        address,
        "PATCH",
        &path,
        Some(TOKEN),
        r#"{"cpu_count":2,"memory_mb":4096}"#,
    )
    .await;
    assert_eq!(changed.status, 200, "{}", changed.text);
    assert_eq!(changed.body["result"]["applied_live"], true);
    assert_eq!(changed.body["result"]["memory"]["mechanism"], "virtio_mem");
    assert_eq!(changed.body["result"]["memory"]["total_mb"], 4_096);
    assert_eq!(
        engine.resizes.lock().expect("resizes").last().copied(),
        Some(ResizeRequest {
            cpu_count: Some(2),
            memory_mb: Some(4_096)
        })
    );

    let read = h2(address, "GET", &path, Some(TOKEN), "").await;
    assert_eq!(read.status, 200, "{}", read.text);
    assert_eq!(
        engine.resizes.lock().expect("resizes").last().copied(),
        Some(ResizeRequest::default())
    );

    *engine.resize_outcome.lock().expect("outcome") = Some(outcome(false));
    let refused = h2(
        address,
        "PATCH",
        &path,
        Some(TOKEN),
        r#"{"memory_mb":4096}"#,
    )
    .await;
    assert_eq!(refused.status, 409, "{}", refused.text);
    assert_eq!(refused.body["error"]["code"], "resize_not_applied");
    assert_eq!(refused.body["result"]["memory"]["state"], "unsupported");

    let invalid = h2(address, "PATCH", &path, Some(TOKEN), "{}").await;
    assert_eq!(invalid.status, 400, "{}", invalid.text);

    // A key nobody published, a spoofed tenant header, and another host's id.
    let before = engine.resizes.lock().expect("resizes").len();
    let anonymous = h2(
        address,
        "PATCH",
        &path,
        Some(OTHER_TOKEN),
        r#"{"cpu_count":1}"#,
    )
    .await;
    assert_eq!(anonymous.status, 401);
    let spoofed = with_headers(
        address,
        b"h2",
        ("GET", &path),
        Some(OTHER_TOKEN),
        &[("x-soma-tenant", "someone-else")],
        "",
    )
    .await;
    assert_eq!(spoofed.status, 401);
    let elsewhere = format!("/api/v1/sandboxes/4{}/resources", &id[1..]);
    let misdirected = h2(address, "GET", &elsewhere, Some(TOKEN), "").await;
    assert_eq!(misdirected.status, 421);
    assert_eq!(engine.resizes.lock().expect("resizes").len(), before);
}
