//! soma-api's own `/filesystem/write` and `/filesystem/read` through `Runner::handle` at sizes
//! past one transfer, against the in-memory guest behind the flow double.

use std::{
    sync::{Arc, atomic::Ordering},
    time::Instant,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;

use super::{
    Runner, RunnerRequest, RunnerResponse,
    flow_tests::{Engine, body_of, call, created_id, runner},
};
use crate::runner::{body::StreamedBody, keys::tests::TOKEN};

async fn created(runner: &Runner) -> String {
    created_id(&call(runner, http::Method::POST, "/api/v1/sandboxes", "{}").await)
}

/// A `/filesystem/write` whose body arrives in `chunk`-byte pieces once the route takes it.
async fn write(runner: &Runner, id: &str, body: Vec<u8>, chunk: usize) -> RunnerResponse {
    let (feeder, streamed) = StreamedBody::channel();
    let feeding = tokio::spawn(async move {
        let Some(sender) = feeder.wait().await else {
            return;
        };
        for piece in body.chunks(chunk) {
            if sender
                .send(Ok(Bytes::copy_from_slice(piece)))
                .await
                .is_err()
            {
                return;
            }
        }
    });
    let response = runner
        .handle(RunnerRequest {
            method: http::Method::POST,
            path: format!("/api/v1/sandboxes/{id}/filesystem/write"),
            query: None,
            authorization: Some(format!("Bearer {TOKEN}")),
            body: Bytes::new(),
            received: Instant::now(),
            forwarded: false,
            upgrade: false,
            content_type: Some("application/json".to_owned()),
            streamed: Some(streamed),
        })
        .await;
    feeding.await.expect("the feeder");
    response
}

/// The whole answer of a read, streamed or not, and whether a streamed one said it was complete.
async fn read(runner: &Runner, id: &str, path: &str) -> (RunnerResponse, Vec<u8>, bool) {
    let mut answer = call(
        runner,
        http::Method::POST,
        &format!("/api/v1/sandboxes/{id}/filesystem/read"),
        &serde_json::json!({ "path": path }).to_string(),
    )
    .await;
    let mut bytes = answer.body.clone();
    let Some(mut stream) = answer.stream.take() else {
        return (answer, bytes, true);
    };
    while let Some(chunk) = stream.recv().await {
        bytes.extend_from_slice(&chunk);
    }
    let complete = answer
        .complete
        .as_ref()
        .is_some_and(|complete| complete.load(Ordering::Acquire));
    (answer, bytes, complete)
}

fn contents(length: usize) -> Vec<u8> {
    (0..length)
        .map(|index| u8::try_from(index * 7 % 251).expect("below 251"))
        .collect()
}

fn temporaries(engine: &Engine) -> usize {
    let guest = engine.guest.lock().expect("guest");
    guest
        .nodes
        .keys()
        .filter(|path| path.contains(".soma-write-"))
        .count()
}

#[tokio::test]
async fn a_file_past_one_transfer_writes_and_reads_back_byte_identical() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let id = created(&runner).await;
    let file = contents(9 * 1024 * 1024 + 5);
    let body = serde_json::json!({"path": "/tmp/big.bin", "content": STANDARD.encode(&file)});

    let written = write(&runner, &id, body.to_string().into_bytes(), 64 * 1024).await;

    assert_eq!(
        written.status,
        200,
        "{}",
        String::from_utf8_lossy(&written.body)
    );
    assert_eq!(body_of(&written)["result"]["byte_length"], file.len());
    assert_eq!(
        engine.guest.lock().expect("guest").file("/tmp/big.bin"),
        Some(&file[..])
    );
    assert_eq!(temporaries(&engine), 0);

    let (answer, bytes, complete) = read(&runner, &id, "/tmp/big.bin").await;
    assert!(complete, "every byte was sent");
    let length = answer
        .headers
        .iter()
        .find(|(name, _)| *name == "content-length")
        .map(|(_, value)| value.clone());
    assert_eq!(
        length,
        Some(bytes.len().to_string()),
        "the stated length is the body's"
    );
    let document: serde_json::Value = serde_json::from_slice(&bytes).expect("one JSON document");
    assert_eq!(document["result"]["content"]["byte_length"], file.len());
    let data = document["result"]["content"]["data"]
        .as_str()
        .expect("data");
    assert_eq!(STANDARD.decode(data).expect("base64"), file);
}

#[tokio::test]
async fn a_small_write_and_read_are_still_the_loopback_route() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let id = created(&runner).await;
    let body = serde_json::json!({"path": "/tmp/small", "content": STANDARD.encode(b"small")});

    let written = write(&runner, &id, body.to_string().into_bytes(), 3).await;
    let (answer, bytes, _) = read(&runner, &id, "/tmp/small").await;

    assert_eq!(written.status, 200);
    assert_eq!(body_of(&written)["result"]["byte_length"], 5);
    assert!(
        answer.stream.is_none(),
        "a file within one transfer is answered whole"
    );
    let document: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON");
    assert_eq!(
        document["result"]["content"]["data"],
        STANDARD.encode(b"small")
    );
    let log = engine.guest.lock().expect("guest").log.clone();
    assert!(
        log.contains(&"write"),
        "the loopback's one whole-file write: {log:?}"
    );
}

#[tokio::test]
async fn a_large_write_that_names_its_path_after_its_content_is_refused_and_leaves_nothing() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let id = created(&runner).await;
    let content = STANDARD.encode(contents(6 * 1024 * 1024));
    let body = format!(r#"{{"content":"{content}","path":"/tmp/late"}}"#);

    let answer = write(&runner, &id, body.into_bytes(), 1024 * 1024).await;

    assert_eq!(answer.status, 413);
    assert_eq!(body_of(&answer)["error"]["code"], "content_too_large");
    assert!(
        engine
            .guest
            .lock()
            .expect("guest")
            .file("/tmp/late")
            .is_none()
    );
    assert_eq!(temporaries(&engine), 0);
}

#[tokio::test]
async fn a_large_write_that_fails_part_way_discards_its_sibling() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let id = created(&runner).await;
    engine
        .guest
        .lock()
        .expect("guest")
        .seed_file("/tmp/keep", b"kept");
    let mut body = format!(
        r#"{{"path":"/tmp/keep","content":"{}"#,
        STANDARD.encode(contents(9 * 1024 * 1024))
    )
    .into_bytes();
    body.extend_from_slice(b"!\"}");

    let answer = write(&runner, &id, body, 512 * 1024).await;

    assert_eq!(
        answer.status,
        400,
        "{}",
        String::from_utf8_lossy(&answer.body)
    );
    assert_eq!(
        engine.guest.lock().expect("guest").file("/tmp/keep"),
        Some(&b"kept"[..])
    );
    assert_eq!(
        temporaries(&engine),
        0,
        "the partly written sibling was discarded"
    );
}

#[tokio::test]
async fn a_large_read_the_guest_stopped_is_never_marked_complete() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let id = created(&runner).await;
    {
        let mut guest = engine.guest.lock().expect("guest");
        guest.seed_file("/tmp/big", &contents(10 * 1024 * 1024));
        // The first transfer and the stat are answered; the sandbox is gone for the next.
        guest.answers_left = Some(2);
    }
    let (answer, _, complete) = read(&runner, &id, "/tmp/big").await;
    assert_eq!(answer.status, 200);
    assert!(!complete);
}
