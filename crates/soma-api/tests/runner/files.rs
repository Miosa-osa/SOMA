//! The platform's file contract over the runner's real transports: an upload streamed in and a
//! download streamed out over HTTP/2 and HTTP/1.1, a download over HTTP/3, an upload refused
//! before its body is sent, and a download the guest stopped reaching the client as an error.

use std::{net::SocketAddr, sync::Arc};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls_pki_types::ServerName;
use soma::{FileAnswer, FileKind, FileOperation, FileRefusal, ManagedFailure};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{
    clients::{client_tls, create_when_ready, h3_request},
    control_plane::start_control_plane,
    facade::{Engine, opener},
    support::{TOKEN, config, scratch, snapshot, start_runner},
};

/// The facade's file answers for the operations an upload and a download use.
pub(crate) fn answer(
    engine: &Engine,
    operation: &FileOperation,
) -> Result<FileAnswer, ManagedFailure> {
    let mut files = engine.files.lock().expect("files");
    Ok(match operation {
        FileOperation::MakeDirectory { .. } | FileOperation::SetMode { .. } => FileAnswer::Done,
        FileOperation::WriteRange {
            path,
            offset,
            bytes,
            ..
        } => {
            let file = files.entry(path.clone()).or_default();
            let start = usize::try_from(*offset).expect("offset");
            file.resize(file.len().max(start + bytes.len()), 0);
            file[start..start + bytes.len()].copy_from_slice(bytes);
            FileAnswer::Written {
                bytes: bytes.len() as u64,
            }
        }
        FileOperation::Rename { path, to } => match files.remove(path) {
            Some(moved) => {
                files.insert(to.clone(), moved);
                FileAnswer::Done
            }
            None => FileAnswer::Refused(FileRefusal::NotFound),
        },
        FileOperation::Stat { path } => match files.get(path) {
            Some(file) => FileAnswer::Stat {
                kind: FileKind::File,
                size: file.len() as u64,
                mode: 0o644,
                modified: 0,
            },
            None => FileAnswer::Refused(FileRefusal::NotFound),
        },
        FileOperation::ReadRange {
            path,
            offset,
            length,
        } => {
            if engine
                .reads_fail_from
                .lock()
                .expect("fail")
                .is_some_and(|from| *offset >= from)
            {
                return Err(ManagedFailure::Backend(
                    soma::BackendFailureKind::Unavailable,
                ));
            }
            let Some(file) = files.get(path) else {
                return Ok(FileAnswer::Refused(FileRefusal::NotFound));
            };
            let start = usize::try_from(*offset).expect("offset").min(file.len());
            let stop = (start + *length as usize).min(file.len());
            FileAnswer::Range {
                bytes: file[start..stop].to_vec(),
                end: stop == file.len(),
            }
        }
        FileOperation::Remove { path, .. } => {
            files.remove(path);
            FileAnswer::Done
        }
        other => unreachable!("an upload or download never asks for {other:?}"),
    })
}

/// A started runner with a sandbox, and the sandbox's id.
async fn sandbox() -> (Arc<Engine>, SocketAddr, String) {
    let (control_plane, state) = start_control_plane().await;
    let engine = Arc::new(Engine::default());
    let handle = start_runner(
        config(control_plane, &scratch("files"), 900),
        opener(&engine),
    )
    .await;
    snapshot(&state, 1);
    let created = create_when_ready(handle.address, "{}").await;
    let id = created.body["id"].as_str().expect("id").to_owned();
    (engine, handle.address, id)
}

fn form(path: &str, filename: &str, contents: &[u8]) -> Vec<u8> {
    let mut body = format!(
        "--b0undary\r\nContent-Disposition: form-data; name=\"path\"\r\n\r\n{path}\r\n--b0undary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(contents);
    body.extend_from_slice(b"\r\n--b0undary--\r\n");
    body
}

/// One request on a fresh TLS connection; the body collected, or the error it ended with.
async fn send(
    address: SocketAddr,
    alpn: &'static [u8],
    (method, path): (&str, &str),
    content_type: &str,
    body: Vec<u8>,
) -> (u16, http::HeaderMap, Result<Bytes, String>) {
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client_tls(&[alpn])));
    let tcp = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect");
    let tls = connector
        .connect(ServerName::try_from("localhost").expect("name"), tcp)
        .await
        .expect("TLS");
    let request = http::Request::builder()
        .method(method)
        .uri(format!("https://localhost{path}"))
        .header(http::header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(http::header::CONTENT_TYPE, content_type)
        .body(Full::new(Bytes::from(body)))
        .expect("request");
    let response = if alpn == b"h2" {
        let (mut sender, connection) =
            hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(tls))
                .await
                .expect("h2 handshake");
        tokio::spawn(connection);
        sender.send_request(request).await.expect("h2 response")
    } else {
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
            .await
            .expect("h1 handshake");
        tokio::spawn(connection);
        sender.send_request(request).await.expect("h1 response")
    };
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .map(http_body_util::Collected::to_bytes)
        .map_err(|error| error.to_string());
    (status, headers, bytes)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_upload_and_a_download_stream_byte_identical_over_http2_and_http1() {
    let (engine, address, id) = sandbox().await;
    let contents: Vec<u8> = (0..9 * 1024 * 1024 + 5_usize)
        .map(|i| u8::try_from(i % 253).expect("byte"))
        .collect();
    let cases: [(&'static [u8], &str); 2] = [(b"h2", "two.bin"), (b"http/1.1", "one.bin")];
    for (alpn, name) in cases {
        let (status, _, body) = send(
            address,
            alpn,
            ("POST", &format!("/api/v1/sandboxes/{id}/fs/upload")),
            "multipart/form-data; boundary=b0undary",
            form("/workspace/", name, &contents),
        )
        .await;
        assert_eq!(
            status,
            201,
            "{:?}",
            body.map(|b| String::from_utf8_lossy(&b).into_owned())
        );

        let target = format!("/api/v1/sandboxes/{id}/fs/download?path=/workspace/{name}");
        let (status, headers, body) = send(
            address,
            alpn,
            ("GET", &target),
            "application/json",
            Vec::new(),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(
            headers[http::header::CONTENT_LENGTH],
            contents.len().to_string().as_str()
        );
        assert_eq!(body.expect("the whole body").as_ref(), contents.as_slice());
    }
    assert!(
        engine
            .files
            .lock()
            .expect("files")
            .keys()
            .all(|path| !path.windows(12).any(|w| w == b".soma-write-")),
        "no sibling outlived its upload"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_download_streams_over_http3() {
    let (engine, address, id) = sandbox().await;
    let text = "line\n".repeat(200_000);
    engine
        .files
        .lock()
        .expect("files")
        .insert(b"/tmp/log.txt".to_vec(), text.clone().into_bytes());
    let answer = h3_request(
        address,
        "GET",
        &format!("/api/v1/sandboxes/{id}/fs/download?path=/tmp/log.txt"),
        TOKEN,
        "",
    )
    .await;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.text, text);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_download_the_guest_stopped_reaches_the_client_as_an_error_not_an_end() {
    let (engine, address, id) = sandbox().await;
    engine
        .files
        .lock()
        .expect("files")
        .insert(b"/tmp/big".to_vec(), vec![1; 9 * 1024 * 1024]);
    *engine.reads_fail_from.lock().expect("fail") = Some(4 * 1024 * 1024);
    let transports: [&'static [u8]; 2] = [b"h2", b"http/1.1"];
    for alpn in transports {
        let target = format!("/api/v1/sandboxes/{id}/fs/download?path=/tmp/big");
        let (status, _, body) = send(
            address,
            alpn,
            ("GET", &target),
            "application/json",
            Vec::new(),
        )
        .await;
        assert_eq!(status, 200);
        assert!(
            body.is_err(),
            "a body short of its length must not end cleanly over {alpn:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_upload_the_runner_refuses_is_answered_before_its_body_is_sent() {
    let (_, address, _) = sandbox().await;
    let elsewhere = format!("9{}", &uuid::Uuid::new_v4().hyphenated().to_string()[1..]);
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client_tls(&[b"http/1.1"])));
    let tcp = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect");
    let mut tls = connector
        .connect(ServerName::try_from("localhost").expect("name"), tcp)
        .await
        .expect("TLS");
    let head = format!(
        "POST /api/v1/sandboxes/{elsewhere}/fs/upload HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: multipart/form-data; boundary=b\r\nContent-Length: 1000000\r\nExpect: 100-continue\r\n\r\n"
    );
    tls.write_all(head.as_bytes()).await.expect("headers");
    let mut answer = vec![0_u8; 512];
    let read = tls.read(&mut answer).await.expect("answer");
    let answer = String::from_utf8_lossy(&answer[..read]);
    assert!(
        answer.starts_with("HTTP/1.1 421"),
        "the final answer came first, with no 100 Continue: {answer}"
    );
}
