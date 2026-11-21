//! The calls, in the order a client makes them, and what each must answer.

use std::time::Instant;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use soma_api::runner::{
    Runner,
    service::{RunnerRequest, RunnerResponse},
};

use super::harness::{request, upload};

/// Larger than two 4 MiB transfers, and not a multiple of anything the transfers use.
const FILE_BYTES: usize = 9 * 1024 * 1024 + 77;
const TEXT: &str = "hello from the runner\n";
const BOUNDARY: &str = "soma-files-proof";

/// Creates a sandbox, drives every file call against it, and destroys it whatever happened.
pub(super) async fn all(runner: &Runner) -> Result<String, String> {
    let created = send(
        runner,
        "create",
        request(http::Method::POST, "/api/v1/sandboxes", "{}"),
    )
    .await;
    expect(&created, 201)?;
    let id = json(&created)["id"]
        .as_str()
        .ok_or("the create named no id")?
        .to_owned();
    let proved = files(runner, &id).await;
    let destroyed = send(
        runner,
        "destroy",
        request(http::Method::DELETE, &format!("/api/v1/sandboxes/{id}"), ""),
    )
    .await;
    let proved = proved?;
    expect(&destroyed, 200)?;
    Ok(proved)
}

async fn files(runner: &Runner, id: &str) -> Result<String, String> {
    let on = |rest: &str| format!("/api/v1/sandboxes/{id}/{rest}");
    text_file(runner, &on).await?;
    let digest = binary_file(runner, id, &on).await?;
    paths(runner, &on).await?;
    Ok(format!(
        "runner files: every call answered as the contract requires; {FILE_BYTES} bytes uploaded, downloaded and read back identical, sha256 {digest} in the guest"
    ))
}

fn post(target: &str, body: &serde_json::Value) -> RunnerRequest {
    request(http::Method::POST, target, &body.to_string())
}

fn get(target: &str) -> RunnerRequest {
    request(http::Method::GET, target, "")
}

/// A directory, a text file written as JSON, listed, and read back.
async fn text_file(runner: &Runner, on: &impl Fn(&str) -> String) -> Result<(), String> {
    let made = post(
        &on("fs/mkdir"),
        &serde_json::json!({"path": "/workspace/e2e/nested"}),
    );
    expect(&send(runner, "mkdir", made).await, 201)?;
    let body = serde_json::json!({"path": "/workspace/e2e/nested/hello.txt", "content": TEXT});
    let written = send(runner, "write", post(&on("fs/write"), &body)).await;
    expect(&written, 201)?;
    let size = serde_json::json!(TEXT.len());
    same("write size", &json(&written)["file"]["size_bytes"], &size)?;

    let listed = send(runner, "list", get(&on("fs?path=/workspace/e2e/nested"))).await;
    expect(&listed, 200)?;
    let entry = &json(&listed)["files"][0];
    same(
        "listed name",
        &entry["name"],
        &serde_json::json!("hello.txt"),
    )?;
    same("listed size", &entry["size_bytes"], &size)?;

    let read = send(
        runner,
        "read",
        get(&on("fs/read?path=/workspace/e2e/nested/hello.txt")),
    )
    .await;
    expect(&read, 200)?;
    same(
        "read back",
        &json(&read)["content"],
        &serde_json::json!(TEXT),
    )
}

/// A binary file larger than two transfers: uploaded, downloaded, read as JSON, and hashed by
/// the guest itself. Returns the digest.
async fn binary_file(
    runner: &Runner,
    id: &str,
    on: &impl Fn(&str) -> String,
) -> Result<String, String> {
    let data = contents();
    let digest = hex(&Sha256::digest(&data));
    let form = form("/workspace/e2e/", "data.bin", &data);
    let uploaded = send(runner, "upload", upload(id, form, BOUNDARY, 64 * 1024)).await;
    expect(&uploaded, 201)?;
    let size = serde_json::json!(FILE_BYTES);
    same("upload size", &json(&uploaded)["file"]["size_bytes"], &size)?;

    let downloaded = download(runner, &on("fs/download?path=/workspace/e2e/data.bin")).await?;
    if downloaded != data {
        return Err(format!(
            "download: {} bytes back, not the {} written",
            downloaded.len(),
            data.len()
        ));
    }

    let read = send(
        runner,
        "read binary",
        get(&on("fs/read?path=/workspace/e2e/data.bin")),
    )
    .await;
    expect(&read, 200)?;
    let decoded = STANDARD
        .decode(json(&read)["content_base64"].as_str().unwrap_or(""))
        .map_err(|_| "read binary: content_base64 is not base64")?;
    if decoded != data {
        return Err("read binary: the bytes differ from those written".to_owned());
    }

    let command = serde_json::json!({"command": "sha256sum /workspace/e2e/data.bin"});
    let hashed = send(runner, "exec sha256sum", post(&on("exec"), &command)).await;
    expect(&hashed, 200)?;
    let stdout = json(&hashed)["data"]["stdout"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    let guest_digest = stdout.split(' ').next().unwrap_or("");
    if guest_digest != digest {
        return Err(format!(
            "the guest hashed {guest_digest}, the runner sent {digest}"
        ));
    }
    Ok(digest)
}

/// A rename, stats either side of it, a mode change, and a recursive removal.
async fn paths(runner: &Runner, on: &impl Fn(&str) -> String) -> Result<(), String> {
    let stat = |path: &str| post(&on("fs/stat"), &serde_json::json!({"path": path}));
    let moved = serde_json::json!({"from": "/workspace/e2e/data.bin", "to": "/workspace/e2e/moved/data.bin"});
    expect(
        &send(runner, "rename", post(&on("fs/rename"), &moved)).await,
        200,
    )?;
    expect(
        &send(runner, "stat old", stat("/workspace/e2e/data.bin")).await,
        404,
    )?;
    let after = send(runner, "stat new", stat("/workspace/e2e/moved/data.bin")).await;
    expect(&after, 200)?;
    same(
        "stat size",
        &json(&after)["data"]["size"],
        &serde_json::json!(FILE_BYTES),
    )?;
    same(
        "stat kind",
        &json(&after)["data"]["is_file"],
        &serde_json::json!(true),
    )?;

    let mode = serde_json::json!({"path": "/workspace/e2e/nested/hello.txt", "mode": "0600"});
    expect(
        &send(runner, "chmod", post(&on("fs/chmod"), &mode)).await,
        200,
    )?;
    let after = send(runner, "stat mode", stat("/workspace/e2e/nested/hello.txt")).await;
    same(
        "stat mode",
        &json(&after)["data"]["mode"],
        &serde_json::json!("0o600"),
    )?;

    let removal = request(
        http::Method::DELETE,
        &on("fs?path=/workspace/e2e&recursive=true"),
        "",
    );
    expect(&send(runner, "delete", removal).await, 200)?;
    expect(
        &send(
            runner,
            "stat deleted",
            stat("/workspace/e2e/nested/hello.txt"),
        )
        .await,
        404,
    )
}

/// Sends one request and prints what it answered.
async fn send(runner: &Runner, name: &str, call: RunnerRequest) -> RunnerResponse {
    let started = Instant::now();
    let answer = runner.handle(call).await;
    println!(
        "{:<16} {} {:>9.3} ms",
        name,
        answer.status,
        started.elapsed().as_secs_f64() * 1_000.0
    );
    answer
}

async fn download(runner: &Runner, target: &str) -> Result<Vec<u8>, String> {
    let started = Instant::now();
    let mut answer = runner.handle(request(http::Method::GET, target, "")).await;
    expect(&answer, 200)?;
    let mut stream = answer
        .stream
        .take()
        .ok_or("download: the answer was not streamed")?;
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.recv().await {
        bytes.extend_from_slice(&chunk);
    }
    let complete = answer
        .complete
        .is_some_and(|complete| complete.load(std::sync::atomic::Ordering::Acquire));
    println!(
        "{:<16} {} {:>9.3} ms, {} bytes, complete {complete}",
        "download",
        answer.status,
        started.elapsed().as_secs_f64() * 1_000.0,
        bytes.len()
    );
    if !complete {
        return Err("download: the body was cut short".to_owned());
    }
    Ok(bytes)
}

fn expect(answer: &RunnerResponse, status: u16) -> Result<(), String> {
    if answer.status == status {
        return Ok(());
    }
    Err(format!(
        "expected {status}, got {}: {}",
        answer.status,
        String::from_utf8_lossy(&answer.body)
    ))
}

fn same(what: &str, got: &serde_json::Value, wanted: &serde_json::Value) -> Result<(), String> {
    if got == wanted {
        Ok(())
    } else {
        Err(format!("{what}: got {got}, wanted {wanted}"))
    }
}

fn json(answer: &RunnerResponse) -> serde_json::Value {
    serde_json::from_slice(&answer.body).unwrap_or(serde_json::Value::Null)
}

/// Bytes no compression or pattern in the path could flatter: a 64-bit xorshift stream.
fn contents() -> Vec<u8> {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    (0..FILE_BYTES)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[0]
        })
        .collect()
}

fn form(path: &str, filename: &str, data: &[u8]) -> Vec<u8> {
    let mut body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"path\"\r\n\r\n{path}\r\n--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    body
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        use std::fmt::Write as _;
        let _written = write!(text, "{byte:02x}");
        text
    })
}
