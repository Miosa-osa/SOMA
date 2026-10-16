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
    // An empty stream sends no event; the fake answers with stderr only.
    assert_eq!(named, vec!["stderr", "exit"]);
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

async fn next_event(events: &mut tokio::sync::mpsc::Receiver<bytes::Bytes>) -> String {
    loop {
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
            .await
            .expect("an event within five seconds")
            .expect("the stream is still open");
        let text = String::from_utf8(chunk.to_vec()).expect("UTF-8");
        // Keepalive comments are not events.
        if !text.starts_with(':') {
            return text;
        }
    }
}

async fn sandbox(runner: &super::Runner) -> String {
    let created = call(runner, http::Method::POST, "/api/v1/sandboxes", "").await;
    let body: serde_json::Value = serde_json::from_slice(&created.body).expect("JSON");
    body["id"].as_str().expect("id").to_owned()
}

#[tokio::test]
async fn output_reaches_the_reader_while_the_command_is_still_running() {
    use super::flow_tests::StreamScript;
    use soma::OutputChannel;

    let engine = Arc::new(Engine::default());
    let (release, gate) = std::sync::mpsc::channel();
    *engine.stream_script.lock().expect("script") = Some(StreamScript {
        chunks: vec![
            (OutputChannel::Stdout, b"first\n".to_vec()),
            (OutputChannel::Stderr, b"later\n".to_vec()),
        ],
        gate,
    });
    let runner = runner(&engine, r#""*""#, "null");
    let id = sandbox(&runner).await;

    let mut streamed = call(
        &runner,
        http::Method::POST,
        &format!("/api/v1/sandboxes/{id}/exec/stream"),
        r#"{"command":"slow"}"#,
    )
    .await;
    let mut events = streamed.stream.take().expect("a streamed body");

    // The command is held open after its first chunk, so this event arrived mid-command.
    assert_eq!(
        next_event(&mut events).await,
        "event: stdout\ndata: {\"text\":\"first\\n\"}\n\n"
    );
    release.send(()).expect("release the command");
    assert_eq!(
        next_event(&mut events).await,
        "event: stderr\ndata: {\"text\":\"later\\n\"}\n\n"
    );
    assert_eq!(
        next_event(&mut events).await,
        "event: exit\ndata: {\"exit_code\":0}\n\n"
    );
}

#[tokio::test]
async fn a_character_split_across_chunks_arrives_whole() {
    use super::flow_tests::StreamScript;
    use soma::OutputChannel;

    let engine = Arc::new(Engine::default());
    let (release, gate) = std::sync::mpsc::channel();
    release.send(()).expect("open the gate");
    let euro = "\u{20ac}".as_bytes();
    *engine.stream_script.lock().expect("script") = Some(StreamScript {
        chunks: vec![
            (OutputChannel::Stdout, euro[..1].to_vec()),
            (OutputChannel::Stdout, euro[1..].to_vec()),
        ],
        gate,
    });
    let runner = runner(&engine, r#""*""#, "null");
    let id = sandbox(&runner).await;
    let mut streamed = call(
        &runner,
        http::Method::POST,
        &format!("/api/v1/sandboxes/{id}/exec/stream"),
        r#"{"command":"euro"}"#,
    )
    .await;
    let mut events = streamed.stream.take().expect("a streamed body");
    let mut text = String::new();
    while let Some(chunk) = events.recv().await {
        text.push_str(std::str::from_utf8(&chunk).expect("UTF-8"));
    }
    assert!(text.contains("event: stdout\ndata: {\"text\":\"\u{20ac}\"}\n\n"));
    assert!(!text.contains('\u{fffd}'));
}

// The journal wait blocks its thread, so the runner's own task needs a thread of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reader_that_leaves_does_not_stop_the_command_or_its_paperwork() {
    use super::flow_tests::StreamScript;
    use soma::OutputChannel;

    let engine = Arc::new(Engine::default());
    let (release, gate) = std::sync::mpsc::channel();
    *engine.stream_script.lock().expect("script") = Some(StreamScript {
        chunks: vec![
            (OutputChannel::Stdout, b"one".to_vec()),
            (OutputChannel::Stdout, b"two".to_vec()),
        ],
        gate,
    });
    let runner = runner(&engine, r#""*""#, "null");
    let id = sandbox(&runner).await;
    let mut streamed = call(
        &runner,
        http::Method::POST,
        &format!("/api/v1/sandboxes/{id}/exec/stream"),
        r#"{"command":"leave"}"#,
    )
    .await;
    let mut events = streamed.stream.take().expect("a streamed body");
    let _first = next_event(&mut events).await;
    drop(events);
    release.send(()).expect("release the command");

    wait_for(runner.journal(), 1);
    let again = call(
        &runner,
        http::Method::POST,
        &format!("/api/v1/sandboxes/{id}/exec"),
        r#"{"command":"true"}"#,
    )
    .await;
    assert_eq!(again.status, 200, "the sandbox was given back");
}
