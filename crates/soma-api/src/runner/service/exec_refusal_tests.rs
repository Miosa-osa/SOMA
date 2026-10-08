//! One exec through the runner's own door, for the two requests a machine must never be asked to run.
//!
//! The reported defect is here: a command over the guest protocol's field bound was admitted, and
//! the engine refused it only after it had taken the operation, which released the sandbox and
//! answered `502 AGENT_UNAVAILABLE`. A refused request now leaves the sandbox exactly where it was,
//! which is the property a caller can see.

use std::sync::{Arc, atomic::Ordering};

use soma::{BackendKind, MachineName, SandboxEntry, SandboxLiveness, SandboxPhase};

use super::{
    Runner,
    flow_tests::{Engine, call, runner},
};
use crate::runner::{ids::SandboxId, keys::tests::TENANT};

/// A sandbox this runner owns is recovered from the state store, so a request for it is addressed
/// here rather than answered with the `421` that names the runner whose tag the id carries.
async fn recovered_runner(engine: &Arc<Engine>) -> (Runner, SandboxId) {
    let runner = runner(engine, r#""*""#, "null");
    let id = SandboxId::mint(runner.config.host_tag);
    engine
        .listed
        .lock()
        .expect("listed")
        .push(SandboxEntry::new(
            id.instance_id().expect("instance"),
            SandboxPhase::Active,
            BackendKind::LinuxKvm,
            Some(MachineName::parse(format!("t-{TENANT}")).expect("label")),
            SandboxLiveness::Live,
        ));
    runner.recover_sandboxes().await;
    (runner, id)
}

#[tokio::test]
async fn an_oversize_command_is_refused_by_name_and_leaves_the_sandbox_usable() {
    let engine = Arc::new(Engine::default());
    let (runner, id) = recovered_runner(&engine).await;
    let path = format!("/api/v1/sandboxes/{id}/exec");

    let command = format!("echo {}", "a".repeat(4_100));
    let refused = call(
        &runner,
        http::Method::POST,
        &path,
        &serde_json::json!({"command": command}).to_string(),
    )
    .await;
    assert_eq!(refused.status, 413);
    let body: serde_json::Value = serde_json::from_slice(&refused.body).expect("JSON");
    assert_eq!(body["error"]["code"], "RUNNER_COMMAND_TOO_LARGE");
    assert_eq!(body["error"]["details"]["field"], "argument");
    assert_eq!(body["error"]["retryable"], false);
    assert_eq!(
        engine.execs.load(Ordering::SeqCst),
        0,
        "no machine may be asked to run a command the protocol will not carry"
    );

    let fine = call(&runner, http::Method::POST, &path, r#"{"command":"true"}"#).await;
    assert_eq!(fine.status, 200, "the sandbox survived the refusal");
    assert_eq!(engine.execs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_deadline_past_the_machine_bound_is_refused_by_name() {
    let engine = Arc::new(Engine::default());
    let (runner, id) = recovered_runner(&engine).await;

    let refused = call(
        &runner,
        http::Method::POST,
        &format!("/api/v1/sandboxes/{id}/exec"),
        r#"{"command":"true","timeout":7200}"#,
    )
    .await;
    assert_eq!(refused.status, 400);
    let body: serde_json::Value = serde_json::from_slice(&refused.body).expect("JSON");
    assert_eq!(body["error"]["code"], "EXEC_TIMEOUT_UNSUPPORTED");
    assert_eq!(engine.execs.load(Ordering::SeqCst), 0);
}
