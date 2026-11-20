//! Uploads and downloads through `Runner::handle`, with the body streamed as the transport
//! streams it and the in-memory guest behind the flow double.

use std::{
    sync::{Arc, atomic::Ordering},
    time::Instant,
};

use bytes::Bytes;

use super::{
    Runner, RunnerRequest, RunnerResponse,
    flow_tests::{Engine, body_of, call, created_id, runner},
};
use crate::runner::{body::StreamedBody, keys::tests::TOKEN};

const FORM: &str = "multipart/form-data; boundary=Bnd";

async fn created(runner: &Runner) -> String {
    created_id(&call(runner, http::Method::POST, "/api/v1/sandboxes", "{}").await)
}

/// A form with a `path` field, when given, and one file part.
fn form(path: Option<&str>, filename: &str, contents: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    if let Some(path) = path {
        body.extend_from_slice(
            format!("--Bnd\r\nContent-Disposition: form-data; name=\"path\"\r\n\r\n{path}\r\n")
                .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--Bnd\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(contents);
    body.extend_from_slice(b"\r\n--Bnd--\r\n");
    body
}

/// Sends `body` as an upload, in chunks of `chunk` bytes, the way the transport hands one on.
async fn upload(
    runner: &Runner,
    id: &str,
    content_type: &str,
    body: Vec<u8>,
    chunk: usize,
) -> RunnerResponse {
    let (feeder, streamed) = StreamedBody::channel();
    let feeding = tokio::spawn(async move {
        // As the transport does, nothing is sent until the route takes the body.
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
            path: format!("/api/v1/sandboxes/{id}/fs/upload"),
            query: None,
            authorization: Some(format!("Bearer {TOKEN}")),
            body: Bytes::new(),
            received: Instant::now(),
            forwarded: false,
            upgrade: false,
            content_type: Some(content_type.to_owned()),
            streamed: Some(streamed),
        })
        .await;
    feeding.await.expect("the feeder");
    response
}

/// Reads a download's whole streamed body, and whether its producer said it was complete.
async fn drained(mut response: RunnerResponse) -> (Vec<u8>, bool) {
    let mut stream = response.stream.take().expect("a download is streamed");
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.recv().await {
        bytes.extend_from_slice(&chunk);
    }
    let complete = response
        .complete
        .is_some_and(|complete| complete.load(Ordering::Acquire));
    (bytes, complete)
}

fn header<'a>(response: &'a RunnerResponse, name: &str) -> Option<&'a str> {
    response
        .headers
        .iter()
        .find(|(header, _)| *header == name)
        .map(|(_, value)| value.as_str())
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
async fn a_file_larger_than_one_transfer_uploads_and_downloads_byte_identical() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let id = created(&runner).await;
    let contents: Vec<u8> = (0..9 * 1024 * 1024 + 13_usize)
        .map(|index| u8::try_from(index % 251).expect("below 251"))
        .collect();

    let uploaded = upload(
        &runner,
        &id,
        FORM,
        form(Some("/workspace/in/"), "data.bin", &contents),
        64 * 1024,
    )
    .await;

    assert_eq!(
        uploaded.status,
        201,
        "{}",
        String::from_utf8_lossy(&uploaded.body)
    );
    assert_eq!(
        body_of(&uploaded),
        serde_json::json!({"file": {"filename": "data.bin", "path": "/workspace/in/data.bin", "size_bytes": contents.len()}, "success": true})
    );
    assert_eq!(temporaries(&engine), 0);

    let download = call(
        &runner,
        http::Method::GET,
        &format!("/api/v1/sandboxes/{id}/fs/download?path=/workspace/in/data.bin"),
        "",
    )
    .await;
    assert_eq!(download.status, 200);
    assert_eq!(
        header(&download, "content-length"),
        Some(contents.len().to_string().as_str())
    );
    assert_eq!(
        header(&download, "content-disposition"),
        Some("attachment; filename=\"data.bin\"")
    );
    assert_eq!(
        header(&download, "content-type"),
        Some("application/octet-stream")
    );
    let (bytes, complete) = drained(download).await;
    assert!(complete, "every byte was sent");
    assert_eq!(bytes, contents);
}

#[tokio::test]
async fn an_upload_that_stops_before_its_closing_delimiter_leaves_nothing_behind() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let id = created(&runner).await;
    let mut body = form(Some("/tmp/"), "a.txt", &vec![b'x'; 5 * 1024 * 1024]);
    body.truncate(body.len() - 12);

    let answer = upload(&runner, &id, FORM, body, 1024 * 1024).await;

    assert_eq!(
        (answer.status, body_of(&answer)["code"].as_str()),
        (400, Some("INVALID_BODY"))
    );
    assert_eq!(
        temporaries(&engine),
        0,
        "the partly written sibling was discarded"
    );
    assert!(
        engine
            .guest
            .lock()
            .expect("guest")
            .file("/tmp/a.txt")
            .is_none()
    );
}

#[tokio::test]
async fn an_upload_without_a_form_or_without_a_file_is_told_so() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let id = created(&runner).await;

    let answer = upload(&runner, &id, "application/json", b"{}".to_vec(), 8).await;
    assert_eq!(
        (answer.status, body_of(&answer)["code"].as_str()),
        (400, Some("MISSING_FILE"))
    );

    let only_path =
        b"--Bnd\r\nContent-Disposition: form-data; name=\"path\"\r\n\r\n/tmp/\r\n--Bnd--\r\n"
            .to_vec();
    let answer = upload(&runner, &id, FORM, only_path, 8).await;
    assert_eq!(
        (answer.status, body_of(&answer)["code"].as_str()),
        (400, Some("MISSING_FILE"))
    );
}

#[tokio::test]
async fn an_upload_to_a_path_outside_the_exposed_trees_writes_nothing() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let id = created(&runner).await;
    let answer = upload(&runner, &id, FORM, form(Some("/etc/"), "passwd", b"x"), 8).await;
    assert_eq!(
        (answer.status, body_of(&answer)["code"].as_str()),
        (400, Some("INVALID_PATH"))
    );
    assert!(engine.guest.lock().expect("guest").log.is_empty());
}

#[tokio::test]
async fn a_download_names_a_missing_path_and_a_directory_before_sending_anything() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let id = created(&runner).await;
    engine
        .guest
        .lock()
        .expect("guest")
        .seed_directory("/workspace/dir");

    let missing = call(
        &runner,
        http::Method::GET,
        &format!("/api/v1/sandboxes/{id}/fs/download?path=/workspace/none"),
        "",
    )
    .await;
    assert_eq!((missing.status, missing.stream.is_none()), (404, true));
    let directory = call(
        &runner,
        http::Method::GET,
        &format!("/api/v1/sandboxes/{id}/files/download?path=/workspace/dir"),
        "",
    )
    .await;
    assert_eq!((directory.status, directory.stream.is_none()), (422, true));
}

#[tokio::test]
async fn a_download_cut_short_says_so_instead_of_ending_as_if_complete() {
    let engine = Arc::new(Engine::default());
    let runner = runner(&engine, r#""*""#, "null");
    let id = created(&runner).await;
    {
        let mut guest = engine.guest.lock().expect("guest");
        guest.seed_file("/tmp/big", &vec![7_u8; 9 * 1024 * 1024]);
        // The stat and the first transfer are answered; the sandbox is gone for the second.
        guest.answers_left = Some(2);
    }

    let download = call(
        &runner,
        http::Method::GET,
        &format!("/api/v1/sandboxes/{id}/fs/download?path=/tmp/big"),
        "",
    )
    .await;
    assert_eq!(download.status, 200);
    let (bytes, complete) = drained(download).await;

    assert_eq!(bytes.len(), soma::MAX_FILE_BYTES);
    assert!(
        !complete,
        "a body short of its stated length is never marked complete"
    );
}
