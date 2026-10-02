//! `exec/stream` through the runner: SSE events, then the exec paperwork.

use std::sync::Arc;

use super::flow_tests::{Engine, call, runner};
use crate::runner::journal::tests::wait_for;

#[tokio::test]
async fn a_streamed_command_ends_with_its_exit_event_and_is_journaled_as_exec() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let created = call(&runner, http::Method::POST, "/api/v1/sandboxes", "").await;
    let body: serde_json::Value = serde_json::from_slice(&created.body).expect("JSON");
    let id = body["id"].as_str().expect("id").to_owned();

    let mut streamed = call(
        &runner,
        http::Method::POST,
        &format!("/api/v1/sandboxes/{id}/exec/stream"),
        r#"{"command":"false"}"#,
    )
    .await;
    assert_eq!(streamed.status, 200);
    assert!(
        streamed
            .headers
            .iter()
            .any(|(name, value)| *name == "content-type" && value == "text/event-stream")
    );
    let mut events = streamed.stream.take().expect("a streamed body");
    let mut text = Vec::new();
    while let Some(chunk) = events.recv().await {
        text.extend_from_slice(&chunk);
    }
    let text = String::from_utf8(text).expect("UTF-8");
    let named: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("event: "))
        .collect();
    assert_eq!(named, vec!["stdout", "stderr", "exit"]);
    assert!(text.contains("event: stderr\ndata: {\"text\":\"no\"}\n\n"));
    assert!(text.ends_with("event: exit\ndata: {\"exit_code\":3}\n\n"));

    // The handler's own create paperwork is the transport's job; the stream journals itself.
    wait_for(runner.journal(), 1);
    let journal = std::fs::read_to_string(runner.journal().path()).expect("journal");
    let line: serde_json::Value =
        serde_json::from_str(journal.lines().next().expect("a line")).expect("JSON");
    assert_eq!(line["kind"], "exec");
    assert_eq!(line["exit_code"], 3);
    assert_eq!(line["status"], 200);

    let again = call(
        &runner,
        http::Method::POST,
        &format!("/api/v1/sandboxes/{id}/exec"),
        r#"{"command":"true"}"#,
    )
    .await;
    assert_eq!(again.status, 200, "the stream gave the sandbox back");
}
