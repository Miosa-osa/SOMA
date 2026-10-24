//! The egress window flow through the runner with a fake facade: open, idempotence, a failed open,
//! close, and the ttl sweep. The fake records every change that reaches the host, so each test can
//! say exactly what the host was asked and in what order.

use std::{sync::Arc, time::Duration};

use soma_local::{EgressSetFailure, EgressSetOutcome};

use super::flow_tests::{Engine, body_of, call, created_id, runner};

const WINDOWED: &str = r#"{"network":{"egress":"denied","windowed":true}}"#;

fn open_body(window: &str, ttl_ms: u64) -> String {
    format!(r#"{{"egress":"internet","ttl_ms":{ttl_ms},"window_id":"{window}"}}"#)
}

fn calls(engine: &Engine) -> Vec<(bool, bool)> {
    engine
        .egress_calls
        .lock()
        .expect("calls")
        .iter()
        .map(|request| (request.open, request.dns))
        .collect()
}

fn script(engine: &Engine, answers: Vec<Result<EgressSetOutcome, EgressSetFailure>>) {
    engine.egress_script.lock().expect("script").extend(answers);
}

async fn windowed_sandbox(engine: &Arc<Engine>) -> (super::Runner, String) {
    let runner = runner(engine, r#""*""#, "null");
    let created = call(&runner, http::Method::POST, "/api/v1/sandboxes", WINDOWED).await;
    assert_eq!(created.status, 201);
    let body = body_of(&created);
    assert_eq!(
        body["network"],
        serde_json::json!({"egress": "denied", "windowed": true}),
        "the create says what the sandbox was given"
    );
    let id = created_id(&created);
    (runner, id)
}

async fn open(
    runner: &super::Runner,
    id: &str,
    window: &str,
    ttl_ms: u64,
) -> super::RunnerResponse {
    call(
        runner,
        http::Method::POST,
        &format!("/api/v1/sandboxes/{id}/network/egress"),
        &open_body(window, ttl_ms),
    )
    .await
}

async fn close(runner: &super::Runner, id: &str, window: &str) -> super::RunnerResponse {
    call(
        runner,
        http::Method::DELETE,
        &format!("/api/v1/sandboxes/{id}/network/egress/{window}"),
        "",
    )
    .await
}

async fn network(runner: &super::Runner, id: &str) -> serde_json::Value {
    let response = call(
        runner,
        http::Method::GET,
        &format!("/api/v1/sandboxes/{id}/network"),
        "",
    )
    .await;
    assert_eq!(response.status, 200);
    body_of(&response)
}

#[tokio::test]
async fn a_window_opens_is_idempotent_on_its_id_and_closes() {
    let engine = Arc::new(Engine::default());
    let (runner, id) = windowed_sandbox(&engine).await;

    let before = network(&runner, &id).await;
    assert_eq!(
        before["base"],
        serde_json::json!({"egress": "denied", "windowed": true})
    );
    assert_eq!(before["current"]["egress"], "denied");
    assert!(before["window_id"].is_null());

    let opened = open(&runner, &id, "w1", 60_000).await;
    assert_eq!(opened.status, 200);
    let body = body_of(&opened);
    assert_eq!(body["window_id"], "w1");
    assert_eq!(body["egress"], "internet");
    assert_eq!(body["changed"], true);
    assert!(body["expires_at"].is_string());
    assert_eq!(calls(&engine), vec![(true, true)]);
    let during = network(&runner, &id).await;
    assert_eq!(during["current"]["egress"], "internet");
    assert_eq!(during["window_id"], "w1");

    // The same id again is the same window: answered, and the host is not asked again.
    let again = open(&runner, &id, "w1", 900_000).await;
    assert_eq!(again.status, 200);
    assert_eq!(body_of(&again)["changed"], false);
    assert_eq!(
        calls(&engine),
        vec![(true, true)],
        "no second change reached the host"
    );

    // Another window cannot open beside it, and cannot close it.
    let other = open(&runner, &id, "w2", 60_000).await;
    assert_eq!(other.status, 409);
    assert_eq!(body_of(&other)["error"]["code"], "egress_window_open");
    let wrong = close(&runner, &id, "w2").await;
    assert_eq!(wrong.status, 409);
    assert_eq!(body_of(&wrong)["error"]["code"], "egress_window_mismatch");
    assert_eq!(
        calls(&engine),
        vec![(true, true)],
        "neither refusal touched the host"
    );

    let closed = close(&runner, &id, "w1").await;
    assert_eq!(closed.status, 200);
    assert_eq!(body_of(&closed)["closed"], true);
    assert_eq!(calls(&engine), vec![(true, true), (false, false)]);
    let after = network(&runner, &id).await;
    assert_eq!(after["current"]["egress"], "denied");
    assert!(after["window_id"].is_null());
}

#[tokio::test]
async fn a_failed_open_closes_again_before_it_answers() {
    let engine = Arc::new(Engine::default());
    let (runner, id) = windowed_sandbox(&engine).await;
    script(&engine, vec![Err(EgressSetFailure::FlipFailed)]);

    let refused = open(&runner, &id, "w1", 60_000).await;
    assert_eq!(refused.status, 502);
    assert_eq!(body_of(&refused)["error"]["code"], "egress_flip_failed");
    // The host was asked to open, then asked to deny before the caller was told.
    assert_eq!(calls(&engine), vec![(true, true), (false, false)]);
    let after = network(&runner, &id).await;
    assert_eq!(after["current"]["egress"], "denied");
    assert!(
        after["window_id"].is_null(),
        "a window that failed to open is not recorded"
    );

    // And the same window id can be tried again.
    assert_eq!(open(&runner, &id, "w1", 60_000).await.status, 200);
}

#[tokio::test]
async fn an_open_the_host_reports_as_closed_is_not_served() {
    let engine = Arc::new(Engine::default());
    let (runner, id) = windowed_sandbox(&engine).await;
    script(
        &engine,
        vec![Ok(EgressSetOutcome {
            open: false,
            dns: false,
            changed: false,
        })],
    );

    let refused = open(&runner, &id, "w1", 60_000).await;
    assert_eq!(
        refused.status, 502,
        "the host's own answer says egress is not open"
    );
    assert_eq!(calls(&engine), vec![(true, true), (false, false)]);
}

#[tokio::test]
async fn a_close_that_is_not_confirmed_is_retried_by_the_sweep() {
    let engine = Arc::new(Engine::default());
    let (runner, id) = windowed_sandbox(&engine).await;
    assert_eq!(open(&runner, &id, "w1", 60_000).await.status, 200);

    // The host does not confirm the close: the caller is told, and the window stays recorded.
    script(&engine, vec![Err(EgressSetFailure::FlipFailed)]);
    let failed = close(&runner, &id, "w1").await;
    assert_eq!(failed.status, 502);
    assert_eq!(network(&runner, &id).await["window_id"], "w1");

    // The sweep tries again and the host confirms.
    runner.reap().await;
    assert_eq!(
        calls(&engine),
        vec![(true, true), (false, false), (false, false)],
        "the sweep asked the host to deny again"
    );
    assert!(network(&runner, &id).await["window_id"].is_null());
}

#[tokio::test]
async fn a_window_is_closed_when_it_runs_out_and_a_failed_close_is_retried() {
    let engine = Arc::new(Engine::default());
    let (runner, id) = windowed_sandbox(&engine).await;
    assert_eq!(open(&runner, &id, "w1", 5).await.status, 200);

    // Not yet due: the sweep leaves a live window alone.
    let live = open(&runner, &id, "w2-not-this", 60_000).await;
    assert_eq!(live.status, 409);
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Due, and the first close fails: the window stays recorded and the sweep keeps trying.
    script(&engine, vec![Err(EgressSetFailure::Unavailable)]);
    runner.reap().await;
    assert_eq!(calls(&engine), vec![(true, true), (false, false)]);
    assert_eq!(
        network(&runner, &id).await["window_id"],
        "w1",
        "still recorded, so still retried"
    );

    runner.reap().await;
    assert_eq!(
        calls(&engine),
        vec![(true, true), (false, false), (false, false)]
    );
    assert!(network(&runner, &id).await["window_id"].is_null());

    // And a closed window is left alone by the next sweep.
    runner.reap().await;
    assert_eq!(calls(&engine).len(), 3);
}

#[tokio::test]
async fn a_sandbox_not_created_windowed_cannot_be_opened_and_the_host_is_not_asked() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let created = call(
        &runner,
        http::Method::POST,
        "/api/v1/sandboxes",
        r#"{"network":{"egress":"denied"}}"#,
    )
    .await;
    assert_eq!(created.status, 201);
    let id = created_id(&created);

    let refused = open(&runner, &id, "w1", 60_000).await;
    assert_eq!(refused.status, 409);
    assert_eq!(body_of(&refused)["error"]["code"], "egress_not_windowed");
    assert!(calls(&engine).is_empty());
    let view = network(&runner, &id).await;
    assert_eq!(
        view["base"],
        serde_json::json!({"egress": "denied", "windowed": false})
    );
}

#[tokio::test]
async fn a_malformed_open_never_reaches_the_host() {
    let engine = Arc::new(Engine::default());
    let (runner, id) = windowed_sandbox(&engine).await;
    for body in [
        "",
        r#"{"egress":"internet","window_id":"w"}"#,
        r#"{"egress":"internet","ttl_ms":900001,"window_id":"w"}"#,
        r#"{"egress":"unrestricted","ttl_ms":1000,"window_id":"w"}"#,
        r#"{"egress":"internet","ttl_ms":1000,"window_id":"has space"}"#,
    ] {
        let response = call(
            &runner,
            http::Method::POST,
            &format!("/api/v1/sandboxes/{id}/network/egress"),
            body,
        )
        .await;
        assert_eq!(response.status, 400, "{body:?}");
    }
    assert!(calls(&engine).is_empty());
}

#[tokio::test]
async fn a_close_with_nothing_recorded_still_asks_the_host_to_deny() {
    let engine = Arc::new(Engine::default());
    let (runner, id) = windowed_sandbox(&engine).await;

    // A runner that lost its record still shuts the window: a close is never skipped.
    let closed = close(&runner, &id, "forgotten").await;
    assert_eq!(closed.status, 200);
    assert_eq!(calls(&engine), vec![(false, false)]);
}
