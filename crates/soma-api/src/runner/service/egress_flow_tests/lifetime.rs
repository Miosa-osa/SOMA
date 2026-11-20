//! A sandbox's network record ends with the sandbox, however the sandbox ends.

use std::{
    sync::{Arc, mpsc},
    time::Instant,
};

use soma::BackendFailureKind;

use crate::runner::{ids::SandboxId, sandboxes::TOMBSTONE_RETENTION};

use super::super::flow_tests::{Engine, call, created_id, runner};

/// A launch that fails after an early answer leaves a tombstone and never reaches a destroy, so
/// nothing on its way out forgets the network record the create made. That record must go when
/// the tombstone does; kept for the life of the runner, every such create would leak one entry.
#[tokio::test]
async fn a_windowed_create_whose_early_launch_fails_is_forgotten_with_its_tombstone() {
    let engine = Arc::new(Engine::default());
    let (release, gate) = mpsc::channel();
    *engine.launch_gate.lock().expect("gate") = Some(gate);
    *engine.launch_refusal.lock().expect("refusal") = Some(BackendFailureKind::Unavailable);
    let runner = runner(&engine, r#""*""#, "null");

    let created = call(
        &runner,
        http::Method::POST,
        "/api/v1/sandboxes",
        r#"{"wait_ready":false,"network":{"egress":"denied","windowed":true}}"#,
    )
    .await;
    assert_eq!(created.status, 201, "the early answer is this create's own");
    let id = SandboxId::parse(&created_id(&created)).expect("a sandbox id");
    assert!(
        runner.windows.is_windowed(&id),
        "the create recorded its network"
    );

    release.send(()).expect("the launch runs and fails");
    // The next call waits for the launch and reads its failure: the record is a tombstone now.
    let failed = call(
        &runner,
        http::Method::POST,
        &format!("/api/v1/sandboxes/{id}/exec"),
        r#"{"command":"false"}"#,
    )
    .await;
    assert_eq!(failed.status, 503);

    // The tombstone goes once its retention has passed, as the reaper's sweep does it.
    runner
        .sandboxes()
        .sweep(Instant::now() + TOMBSTONE_RETENTION);
    assert!(!runner.sandboxes().holds(&id), "the tombstone is gone");
    runner.reap().await;
    assert!(
        runner.windows.base(&id).is_none(),
        "the network record outlived its sandbox"
    );
}
