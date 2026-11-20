use std::{io, net::SocketAddr, sync::Arc, time::Instant};

use bytes::{Buf, Bytes, BytesMut};

use crate::runner::{
    body::{self, StreamedBody},
    limits::{
        BODY_READ_TIMEOUT, IDLE_TIMEOUT, MAX_HEADER_BYTES, MAX_STREAMS_PER_CONNECTION, PerIp,
    },
    service::{Runner, RunnerRequest, RunnerResponse},
    tls::CertificateStore,
    transport::{
        apply_headers, authorization, journaled::cut_short, payload_too_large, request_timeout,
    },
};

/// Binds the HTTP/3 endpoint on the public address.
///
/// # Errors
///
/// Returns the TLS configuration or UDP bind failure.
pub fn bind_quic(
    address: SocketAddr,
    certificates: &Arc<CertificateStore>,
) -> io::Result<quinn::Endpoint> {
    let tls = certificates.server_config(&[b"h3"], true)?;
    let crypto =
        quinn::crypto::rustls::QuicServerConfig::try_from(tls).map_err(io::Error::other)?;
    let mut server = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    let mut transport = quinn::TransportConfig::default();
    transport.max_concurrent_bidi_streams(MAX_STREAMS_PER_CONNECTION.into());
    transport.max_idle_timeout(Some(IDLE_TIMEOUT.try_into().map_err(io::Error::other)?));
    server.transport_config(Arc::new(transport));
    quinn::Endpoint::server(server, address)
}

/// Serves HTTP/3 until the endpoint closes.
pub async fn serve_quic(endpoint: quinn::Endpoint, runner: Arc<Runner>, per_ip: Arc<PerIp>) {
    while let Some(incoming) = endpoint.accept().await {
        let Some(slot) = per_ip.admit(incoming.remote_address().ip()) else {
            incoming.refuse();
            continue;
        };
        let runner = Arc::clone(&runner);
        tokio::spawn(async move {
            let _slot = slot;
            let Ok(connection) = incoming.await else {
                return;
            };
            let Ok(mut connection) = h3::server::builder()
                .max_field_section_size(u64::from(MAX_HEADER_BYTES))
                .build::<_, Bytes>(h3_quinn::Connection::new(connection))
                .await
            else {
                return;
            };
            while let Ok(Some(resolver)) = connection.accept().await {
                let runner = Arc::clone(&runner);
                tokio::spawn(async move {
                    let _answered = serve_h3_request(resolver, &runner).await;
                });
            }
        });
    }
}

async fn serve_h3_request(
    resolver: h3::server::RequestResolver<h3_quinn::Connection, Bytes>,
    runner: &Runner,
) -> Result<(), h3::error::StreamError> {
    let received = Instant::now();
    let (request, stream) = resolver.resolve_request().await?;
    let (mut stream, mut incoming) = stream.split();
    let authorization = authorization(request.headers());
    let (method, path, key) = (
        request.method(),
        request.uri().path(),
        authorization.as_deref(),
    );
    // An upload is handed on as it arrives; every other body is read whole first.
    let read = if let Some(limit) = runner.streamed_limit(method, path, key) {
        let (sender, streamed) = StreamedBody::channel();
        tokio::spawn(body::pump_h3(incoming, sender, limit));
        Some(Ok((Bytes::new(), Some(streamed))))
    } else {
        let limit = runner.body_limit(method, path, key);
        match tokio::time::timeout(BODY_READ_TIMEOUT, read_body(&mut incoming, limit)).await {
            Ok(Ok(body)) => Some(body.map(|body| (body, None))),
            Ok(Err(error)) => return Err(error),
            Err(_) => None,
        }
    };
    let content_type = request
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let mut response = match read {
        Some(Ok((body, streamed))) => {
            let (parts, ()) = request.into_parts();
            runner
                .handle(RunnerRequest {
                    method: parts.method,
                    path: parts.uri.path().to_owned(),
                    query: parts.uri.query().map(str::to_owned),
                    authorization,
                    body,
                    received,
                    forwarded: false,
                    // HTTP/3 has no `Upgrade`; a tunnel needs HTTP/1.1.
                    upgrade: false,
                    content_type,
                    streamed,
                })
                .await
        }
        Some(Err(refused)) => refused,
        None => request_timeout(),
    };
    let mut head = http::Response::new(());
    *head.status_mut() = http::StatusCode::from_u16(response.status)
        .unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR);
    apply_headers(head.headers_mut(), &response.headers);
    let entry = response.journal.take();
    let body = Bytes::from(std::mem::take(&mut response.body));
    let mut rest = response.stream.take();
    let complete = response.complete.take();
    let sent = async {
        stream.send_response(head).await?;
        if !body.is_empty() || rest.is_none() {
            stream.send_data(body).await?;
        }
        if let Some(rest) = rest.as_mut() {
            while let Some(chunk) = rest.recv().await {
                stream.send_data(chunk).await?;
            }
        }
        // A stream its producer cut short is reset rather than finished, as on TCP.
        if cut_short(complete.as_ref()) {
            stream.stop_stream(h3::error::Code::H3_INTERNAL_ERROR);
            return Ok(());
        }
        stream.finish().await
    }
    .await;
    // Journaled once the answer is out, or once sending it failed: as on TCP, the request was
    // served either way.
    if let Some(entry) = entry {
        runner.journal().record(entry);
    }
    sent
}

/// Reads a request body up to the cap; an oversized body is answered, not read to the end.
async fn read_body(
    stream: &mut h3::server::RequestStream<h3_quinn::RecvStream, Bytes>,
    limit: usize,
) -> Result<Result<Bytes, RunnerResponse>, h3::error::StreamError> {
    let mut body = BytesMut::new();
    while let Some(mut chunk) = stream.recv_data().await? {
        if body.len() + chunk.remaining() > limit {
            return Ok(Err(payload_too_large()));
        }
        while chunk.has_remaining() {
            let piece = chunk.chunk();
            body.extend_from_slice(piece);
            let read = piece.len();
            chunk.advance(read);
        }
    }
    Ok(Ok(body.freeze()))
}
