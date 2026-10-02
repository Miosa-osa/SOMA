//! Public clients over HTTP/2, HTTP/1.1, and HTTP/3.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use bytes::{Buf, Bytes};
use http_body_util::{BodyExt, Full};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls_pki_types::ServerName;

use crate::support::{TOKEN, provider, roots};

pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) headers: http::HeaderMap,
    pub(crate) body: serde_json::Value,
}

pub(crate) fn client_tls(alpn: &[&[u8]]) -> rustls::ClientConfig {
    let mut tls = rustls::ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("versions")
        .with_root_certificates(roots())
        .with_no_client_auth();
    tls.alpn_protocols = alpn.iter().map(|protocol| protocol.to_vec()).collect();
    tls
}

/// One HTTP/2 or HTTP/1.1 request on a fresh TLS connection.
pub(crate) async fn tcp_request(
    address: SocketAddr,
    alpn: &'static [u8],
    method: &str,
    path: &str,
    token: Option<&str>,
    body: &str,
) -> Answer {
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client_tls(&[alpn])));
    let tcp = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect");
    let tls = connector
        .connect(ServerName::try_from("localhost").expect("name"), tcp)
        .await
        .expect("TLS");
    let negotiated = tls.get_ref().1.alpn_protocol().map(<[u8]>::to_vec);
    assert_eq!(negotiated.as_deref(), Some(alpn));
    let mut request = http::Request::builder()
        .method(method)
        .uri(format!("https://localhost{path}"))
        .header(http::header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        request = request.header(http::header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let request = request
        .body(Full::new(Bytes::from(body.to_owned())))
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
        .expect("body")
        .to_bytes();
    Answer {
        status,
        headers,
        body: serde_json::from_slice(&bytes).expect("every runner answer is JSON"),
    }
}

pub(crate) async fn h2(
    address: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: &str,
) -> Answer {
    tcp_request(address, b"h2", method, path, token, body).await
}

/// One HTTP/3 request on a fresh QUIC connection.
pub(crate) async fn h3_request(
    address: SocketAddr,
    method: &str,
    path: &str,
    token: &str,
    body: &str,
) -> Answer {
    let mut endpoint =
        quinn::Endpoint::client("127.0.0.1:0".parse().expect("address")).expect("endpoint");
    let crypto =
        quinn::crypto::rustls::QuicClientConfig::try_from(client_tls(&[b"h3"])).expect("QUIC TLS");
    endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(crypto)));
    let connection = endpoint
        .connect(address, "localhost")
        .expect("connect")
        .await
        .expect("QUIC handshake");
    let (mut driver, mut sender) = h3::client::new(h3_quinn::Connection::new(connection))
        .await
        .expect("h3 handshake");
    tokio::spawn(async move { std::future::poll_fn(|context| driver.poll_close(context)).await });
    let request = http::Request::builder()
        .method(method)
        .uri(format!("https://localhost{path}"))
        .header(http::header::AUTHORIZATION, format!("Bearer {token}"))
        .body(())
        .expect("request");
    let mut stream = sender.send_request(request).await.expect("send");
    stream
        .send_data(Bytes::from(body.to_owned()))
        .await
        .expect("send body");
    stream.finish().await.expect("finish");
    let response = stream.recv_response().await.expect("response");
    let mut bytes = Vec::new();
    while let Some(mut chunk) = stream.recv_data().await.expect("data") {
        while chunk.has_remaining() {
            let piece = chunk.chunk();
            bytes.extend_from_slice(piece);
            let read = piece.len();
            chunk.advance(read);
        }
    }
    Answer {
        status: response.status().as_u16(),
        headers: response.headers().clone(),
        body: serde_json::from_slice(&bytes).expect("JSON"),
    }
}

/// Creates until the snapshot has landed and the create succeeds.
pub(crate) async fn create_when_ready(address: SocketAddr, body: &str) -> Answer {
    for _ in 0..200 {
        let answer = h2(address, "POST", "/api/v1/sandboxes", Some(TOKEN), body).await;
        if answer.status == 201 {
            return answer;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the create never succeeded after the snapshot");
}
