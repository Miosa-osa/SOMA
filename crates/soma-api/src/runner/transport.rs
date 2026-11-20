use std::{
    convert::Infallible,
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use http_body_util::{BodyExt, Limited};
use hyper::body::Incoming;
use hyper_util::{
    rt::{TokioExecutor, TokioIo, TokioTimer},
    server::conn::auto,
};

use crate::runner::{
    body::{self, StreamedBody},
    journal::Journal,
    limits::{
        Activity, BODY_READ_TIMEOUT, HEADER_READ_TIMEOUT, IDLE_CHECK, IDLE_TIMEOUT, IdleIo,
        MAX_HEADER_BYTES, MAX_HTTP1_BUFFER_BYTES, MAX_STREAMS_PER_CONNECTION, PerIp,
        TLS_HANDSHAKE_TIMEOUT,
    },
    service::{Runner, RunnerRequest, RunnerResponse, TunnelHandoff},
    tls::CertificateStore,
};
use crate::tunnel;

mod early;
pub(crate) mod journaled;

pub(crate) use early::{payload_too_large, request_timeout};
pub use journaled::JournaledBody;

const CONTENT_TYPE: &str = "application/json; charset=utf-8";

/// Serves TLS over TCP, negotiating HTTP/2 or HTTP/1.1, until the listener fails.
pub async fn serve_tcp(
    listener: tokio::net::TcpListener,
    certificates: Arc<CertificateStore>,
    runner: Arc<Runner>,
    max_connections: usize,
    per_ip: Arc<PerIp>,
) {
    let tls = match certificates.server_config(&[b"h2", b"http/1.1"], false) {
        Ok(config) => tokio_rustls::TlsAcceptor::from(Arc::new(config)),
        Err(error) => {
            eprintln!("soma-api: runner TCP listener could not start: {error}");
            return;
        }
    };
    let connections = Arc::new(tokio::sync::Semaphore::new(max_connections));
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                // Running out of descriptors is transient; anything else is logged the same way
                // and retried after a pause rather than ending the listener.
                eprintln!("soma-api: runner accept failed: {error}");
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let Ok(permit) = Arc::clone(&connections).try_acquire_owned() else {
            continue;
        };
        let Some(slot) = per_ip.admit(peer.ip()) else {
            continue;
        };
        let (tls, runner) = (tls.clone(), Arc::clone(&runner));
        tokio::spawn(async move {
            let _held = (permit, slot);
            if stream.set_nodelay(true).is_err() {
                return;
            }
            let Ok(Ok(stream)) =
                tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, tls.accept(stream)).await
            else {
                return;
            };
            let activity = Activity::new();
            serve_http(
                TokioIo::new(IdleIo::new(stream, &activity)),
                &activity,
                runner,
                false,
            )
            .await;
        });
    }
}

/// Serves HTTP/1.1 or HTTP/2 on one established connection until it closes or goes idle.
///
/// An idle connection is shut down gracefully: requests in flight (a long command, say) still
/// finish, and no new one is accepted.
///
/// `forwarded` marks the private listener: its requests come from other runners and are never
/// forwarded again.
pub async fn serve_http<I>(io: I, activity: &Activity, runner: Arc<Runner>, forwarded: bool)
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let service = hyper::service::service_fn(move |request: http::Request<Incoming>| {
        let runner = Arc::clone(&runner);
        async move { Ok::<_, Infallible>(respond(&runner, request, forwarded).await) }
    });
    let mut builder = auto::Builder::new(TokioExecutor::new());
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_READ_TIMEOUT)
        .max_buf_size(MAX_HTTP1_BUFFER_BYTES);
    builder
        .http2()
        .timer(TokioTimer::new())
        .max_concurrent_streams(MAX_STREAMS_PER_CONNECTION)
        .max_header_list_size(MAX_HEADER_BYTES);
    let connection = builder.serve_connection_with_upgrades(io, service);
    tokio::pin!(connection);
    let mut check = tokio::time::interval(IDLE_CHECK);
    loop {
        tokio::select! {
            _closed = connection.as_mut() => return,
            _tick = check.tick() => {
                if activity.idle_for() >= IDLE_TIMEOUT {
                    connection.as_mut().graceful_shutdown();
                    let _closed = connection.as_mut().await;
                    return;
                }
            }
        }
    }
}

async fn respond(
    runner: &Runner,
    mut request: http::Request<Incoming>,
    forwarded: bool,
) -> http::Response<JournaledBody> {
    let received = Instant::now();
    // A tunnel is HTTP/1.1 only: HTTP/2 has no `Upgrade`, so such a request is answered `426`.
    let upgrade = request.version() == http::Version::HTTP_11
        && tunnel::asks_for_upgrade(
            header_text(request.headers(), http::header::CONNECTION),
            header_text(request.headers(), http::header::UPGRADE),
        );
    let on_upgrade = upgrade.then(|| hyper::upgrade::on(&mut request));
    let (parts, body) = request.into_parts();
    let authorization = authorization(&parts.headers);
    let (path, key) = (parts.uri.path(), authorization.as_deref());
    // An upload is handed on as it arrives; every other body is read whole first.
    let (body, streamed) = if let Some(limit) = runner.streamed_limit(&parts.method, path, key) {
        let (sender, streamed) = StreamedBody::channel();
        tokio::spawn(body::pump_hyper(body, sender, limit));
        (Bytes::new(), Some(streamed))
    } else {
        let limited = Limited::new(body, runner.body_limit(&parts.method, path, key));
        match tokio::time::timeout(BODY_READ_TIMEOUT, limited.collect()).await {
            Ok(Ok(collected)) => (collected.to_bytes(), None),
            Ok(Err(_)) => return encode_hyper(payload_too_large(), None),
            Err(_) => return encode_hyper(request_timeout(), None),
        }
    };
    let content_type = header_text(&parts.headers, http::header::CONTENT_TYPE).map(str::to_owned);
    let mut response = runner
        .handle(RunnerRequest {
            method: parts.method,
            path: parts.uri.path().to_owned(),
            query: parts.uri.query().map(str::to_owned),
            authorization,
            body,
            received,
            forwarded,
            upgrade,
            content_type,
            streamed,
        })
        .await;
    if let (Some(handoff), Some(on_upgrade)) = (response.tunnel.take(), on_upgrade) {
        return switch_protocols(handoff, on_upgrade);
    }
    encode_hyper(response, Some(runner.journal()))
}

fn header_text(headers: &http::HeaderMap, name: http::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// Answers `101` and pumps the guest connection over the upgraded one once hyper hands it over.
///
/// The pump holds the tunnel's place in the table until it ends, whichever side ends it.
fn switch_protocols(
    handoff: TunnelHandoff,
    on_upgrade: hyper::upgrade::OnUpgrade,
) -> http::Response<JournaledBody> {
    tokio::spawn(async move {
        let TunnelHandoff {
            guest,
            slot,
            idle,
            touch,
        } = handoff;
        if let Ok(upgraded) = on_upgrade.await {
            tunnel::pump(TokioIo::new(upgraded), guest, idle, &*touch).await;
        }
        drop(slot);
    });
    let mut response = http::Response::new(JournaledBody {
        head: None,
        stream: None,
        after: None,
        complete: None,
    });
    *response.status_mut() = http::StatusCode::SWITCHING_PROTOCOLS;
    let headers = response.headers_mut();
    headers.insert(
        http::header::CONNECTION,
        http::HeaderValue::from_static("upgrade"),
    );
    headers.insert(
        http::header::UPGRADE,
        http::HeaderValue::from_static(tunnel::UPGRADE_TOKEN),
    );
    response
}

fn encode_hyper(
    mut response: RunnerResponse,
    journal: Option<&Journal>,
) -> http::Response<JournaledBody> {
    let after = journal.and_then(|journal| {
        response
            .journal
            .take()
            .map(|entry| (journal.clone(), entry))
    });
    let mut built = http::Response::new(JournaledBody {
        head: Some(Bytes::from(std::mem::take(&mut response.body))),
        stream: response.stream.take(),
        after,
        complete: response.complete.take(),
    });
    *built.status_mut() = http::StatusCode::from_u16(response.status)
        .unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR);
    apply_headers(built.headers_mut(), &response.headers);
    built
}

pub(crate) fn apply_headers(headers: &mut http::HeaderMap, extra: &[(&'static str, String)]) {
    headers.insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static(CONTENT_TYPE),
    );
    headers.insert(
        http::header::CACHE_CONTROL,
        http::HeaderValue::from_static("no-store"),
    );
    for (name, value) in extra {
        if let Ok(value) = http::HeaderValue::from_str(value) {
            headers.insert(*name, value);
        }
    }
}

pub(crate) fn authorization(headers: &http::HeaderMap) -> Option<String> {
    headers
        .get(http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}
