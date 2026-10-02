use std::{io, net::SocketAddr, sync::Arc, time::Duration, time::Instant};

use bytes::{Buf, Bytes, BytesMut};

use crate::runner::{
    service::{MAX_BODY_BYTES, Runner, RunnerRequest},
    tls::CertificateStore,
    transport::{MAX_STREAMS_PER_CONNECTION, apply_headers, authorization, payload_too_large},
};

const QUIC_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

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
    transport.max_idle_timeout(Some(
        QUIC_IDLE_TIMEOUT.try_into().map_err(io::Error::other)?,
    ));
    server.transport_config(Arc::new(transport));
    quinn::Endpoint::server(server, address)
}

/// Serves HTTP/3 until the endpoint closes.
pub async fn serve_quic(endpoint: quinn::Endpoint, runner: Arc<Runner>) {
    while let Some(incoming) = endpoint.accept().await {
        let runner = Arc::clone(&runner);
        tokio::spawn(async move {
            let Ok(connection) = incoming.await else {
                return;
            };
            let Ok(mut connection) = h3::server::builder()
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
    let (request, mut stream) = resolver.resolve_request().await?;
    let mut body = BytesMut::new();
    let mut too_large = false;
    while let Some(mut chunk) = stream.recv_data().await? {
        if body.len() + chunk.remaining() > MAX_BODY_BYTES {
            too_large = true;
            break;
        }
        while chunk.has_remaining() {
            let piece = chunk.chunk();
            body.extend_from_slice(piece);
            let read = piece.len();
            chunk.advance(read);
        }
    }
    let mut response = if too_large {
        payload_too_large()
    } else {
        let (parts, ()) = request.into_parts();
        runner
            .handle(RunnerRequest {
                method: parts.method,
                path: parts.uri.path().to_owned(),
                authorization: authorization(&parts.headers),
                body: body.freeze(),
                received,
            })
            .await
    };
    let mut head = http::Response::new(());
    *head.status_mut() = http::StatusCode::from_u16(response.status)
        .unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR);
    apply_headers(head.headers_mut(), &response.headers);
    let entry = response.journal.take();
    let body = Bytes::from(std::mem::take(&mut response.body));
    let sent = async {
        stream.send_response(head).await?;
        stream.send_data(body).await?;
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
