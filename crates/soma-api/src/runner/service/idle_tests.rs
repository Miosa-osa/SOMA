//! C7 idle timeouts through the runner: activity resets the timer, PATCH replaces it.

use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use super::flow_tests::{Engine, call, reap_until_destroyed, runner};

fn id_of(response: &super::RunnerResponse) -> String {
    let body: serde_json::Value = serde_json::from_slice(&response.body).expect("JSON");
    body["id"].as_str().expect("id").to_owned()
}

#[tokio::test]
async fn every_authenticated_call_resets_the_idle_timer() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let created = call(
        &runner,
        http::Method::POST,
        "/api/v1/sandboxes",
        r#"{"timeout":1}"#,
    )
    .await;
    let id = id_of(&created);

    tokio::time::sleep(Duration::from_millis(700)).await;
    let exec = call(
        &runner,
        http::Method::POST,
        &format!("/api/v1/sandboxes/{id}/exec"),
        r#"{"command":"true"}"#,
    )
    .await;
    assert_eq!(exec.status, 200);
    tokio::time::sleep(Duration::from_millis(700)).await;
    runner.reap().await;
    assert_eq!(
        engine.destroys.load(Ordering::SeqCst),
        0,
        "the exec reset the timer"
    );

    tokio::time::sleep(Duration::from_millis(400)).await;
    runner.reap().await;
    assert_eq!(engine.destroys.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn patch_replaces_the_idle_timeout_and_zero_means_none() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let created = call(
        &runner,
        http::Method::POST,
        "/api/v1/sandboxes",
        r#"{"timeout":1}"#,
    )
    .await;
    let id = id_of(&created);

    let patched = call(
        &runner,
        http::Method::PATCH,
        &format!("/api/v1/sandboxes/{id}"),
        r#"{"timeout":0}"#,
    )
    .await;
    assert_eq!(patched.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&patched.body).expect("JSON");
    assert_eq!(
        body,
        serde_json::json!({"expires_at": null, "id": id, "timeout_sec": 0})
    );
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    runner.reap().await;
    assert_eq!(engine.destroys.load(Ordering::SeqCst), 0);

    let extended = call(
        &runner,
        http::Method::PATCH,
        &format!("/api/v1/sandboxes/{id}"),
        r#"{"timeout":600}"#,
    )
    .await;
    let body: serde_json::Value = serde_json::from_slice(&extended.body).expect("JSON");
    assert_eq!(body["timeout_sec"], 600);
    assert!(body["expires_at"].is_string());
    for bad in [r#"{"timeout":-1}"#, r#"{"timeout":86401}"#, "{}"] {
        let refused = call(
            &runner,
            http::Method::PATCH,
            &format!("/api/v1/sandboxes/{id}"),
            bad,
        )
        .await;
        assert_eq!(refused.status, 400, "{bad}");
    }
}

#[tokio::test]
async fn a_command_longer_than_the_idle_timeout_holds_its_sandbox() {
    let engine = Arc::new(Engine::default());
    *engine.exec_delay.lock().expect("delay") = Duration::from_secs(3);
    let runner = Arc::new(runner(&engine, r#""*""#, "null"));
    let created = call(
        &runner,
        http::Method::POST,
        "/api/v1/sandboxes",
        r#"{"timeout":1}"#,
    )
    .await;
    let id = id_of(&created);

    let running = {
        let (runner, path) = (Arc::clone(&runner), format!("/api/v1/sandboxes/{id}/exec"));
        tokio::spawn(async move {
            call(
                &runner,
                http::Method::POST,
                &path,
                r#"{"command":"sleep 3"}"#,
            )
            .await
        })
    };
    // Three times the idle timeout passes with the command in flight; every sweep skips it.
    for _ in 0..6 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        runner.reap().await;
        assert_eq!(
            engine.destroys.load(Ordering::SeqCst),
            0,
            "busy is never idle"
        );
    }
    let finished = running.await.expect("exec task");
    assert_eq!(finished.status, 200);

    // The timer restarted when the command ended: still alive just after, gone once idle.
    runner.reap().await;
    assert_eq!(engine.destroys.load(Ordering::SeqCst), 0);
    reap_until_destroyed(&runner, &engine).await;
    assert_eq!(engine.destroys.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_terminal_read_longer_than_the_idle_timeout_holds_its_sandbox_without_blocking_exec() {
    let engine = Arc::new(Engine::default());
    *engine.terminal_delay.lock().expect("delay") = Duration::from_secs(3);
    let runner = Arc::new(runner(&engine, r#""*""#, "null"));
    let created = call(
        &runner,
        http::Method::POST,
        "/api/v1/sandboxes",
        r#"{"timeout":1}"#,
    )
    .await;
    let id = id_of(&created);

    let reading = {
        let (runner, path) = (
            Arc::clone(&runner),
            format!("/api/v1/sandboxes/{id}/terminal/read"),
        );
        tokio::spawn(async move {
            call(&runner, http::Method::POST, &path, r#"{"wait_ms":3000}"#).await
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    // A command overlaps the read: the hold is a counter, not the command's exclusive slot.
    let exec = call(
        &runner,
        http::Method::POST,
        &format!("/api/v1/sandboxes/{id}/exec"),
        r#"{"command":"true"}"#,
    )
    .await;
    assert_eq!(exec.status, 200, "an exec runs while the read is held");
    for _ in 0..6 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        runner.reap().await;
        assert_eq!(
            engine.destroys.load(Ordering::SeqCst),
            0,
            "a held sandbox is never idle"
        );
    }
    let read = reading.await.expect("read task");
    assert_eq!(read.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&read.body).expect("JSON");
    assert_eq!(body["operation"], "sandbox.terminal");

    // The read's end restarted the timer.
    runner.reap().await;
    assert_eq!(engine.destroys.load(Ordering::SeqCst), 0);
    reap_until_destroyed(&runner, &engine).await;
    assert_eq!(engine.destroys.load(Ordering::SeqCst), 1);
}
