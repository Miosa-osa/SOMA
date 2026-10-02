use std::{
    convert::Infallible,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use bytes::Bytes;
use http_body_util::{BodyExt, Limited};
use hyper::body::{Body, Frame, Incoming, SizeHint};
use hyper_util::{
    rt::{TokioExecutor, TokioIo, TokioTimer},
    server::conn::auto,
};

use crate::runner::{
    journal::{Entry, Journal},
    limits::{
        Activity, BODY_READ_TIMEOUT, HEADER_READ_TIMEOUT, IDLE_CHECK, IDLE_TIMEOUT, IdleIo,
        MAX_HEADER_BYTES, MAX_HTTP1_BUFFER_BYTES, MAX_STREAMS_PER_CONNECTION, PerIp,
        TLS_HANDSHAKE_TIMEOUT,
    },
    public_wire,
    service::{MAX_BODY_BYTES, Runner, RunnerRequest, RunnerResponse},
    tls::CertificateStore,
};

const CONTENT_TYPE: &str = "application/json; charset=utf-8";

/// A response body that journals its request once the body is gone.
///
/// Hyper drops the body after its last frame is written, or when the connection fails first, so
/// the journal entry is written strictly after the response left this process: the paperwork is
/// never on the request path (contract C4).
///
/// A streamed answer (server-sent events, a forwarded response) sends its first bytes, then
/// every chunk its producer sends, and ends when the producer drops its sender.
pub struct JournaledBody {
    head: Option<Bytes>,
    stream: Option<tokio::sync::mpsc::Receiver<Bytes>>,
    after: Option<(Journal, Entry)>,
}

impl Body for JournaledBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        if let Some(head) = self.head.take().filter(|head| !head.is_empty()) {
            return Poll::Ready(Some(Ok(Frame::data(head))));
        }
        match self.stream.as_mut() {
            Some(stream) => stream
                .poll_recv(context)
                .map(|chunk| chunk.map(|bytes| Ok(Frame::data(bytes)))),
            None => Poll::Ready(None),
        }
    }

    fn is_end_stream(&self) -> bool {
        self.stream.is_none() && self.head.as_ref().is_none_or(Bytes::is_empty)
    }

    fn size_hint(&self) -> SizeHint {
        match (&self.stream, &self.head) {
            (None, head) => SizeHint::with_exact(head.as_ref().map_or(0, |head| head.len() as u64)),
            (Some(_), _) => SizeHint::default(),
        }
    }
}

impl Drop for JournaledBody {
    fn drop(&mut self) {
        if let Some((journal, entry)) = self.after.take() {
            journal.record(entry);
        }
    }
}

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
    let connection = builder.serve_connection(io, service);
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
    request: http::Request<Incoming>,
    forwarded: bool,
) -> http::Response<JournaledBody> {
    let received = Instant::now();
    let (parts, body) = request.into_parts();
    let collected = match tokio::time::timeout(
        BODY_READ_TIMEOUT,
        Limited::new(body, MAX_BODY_BYTES).collect(),
    )
    .await
    {
        Ok(Ok(collected)) => collected,
        Ok(Err(_)) => return encode_hyper(payload_too_large(), None),
        Err(_) => return encode_hyper(request_timeout(), None),
    };
    let response = runner
        .handle(RunnerRequest {
            method: parts.method,
            path: parts.uri.path().to_owned(),
            authorization: authorization(&parts.headers),
            body: collected.to_bytes(),
            received,
            forwarded,
        })
        .await;
    encode_hyper(response, Some(runner.journal()))
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

pub(crate) fn request_timeout() -> RunnerResponse {
    let mut response = payload_too_large();
    response.status = 408;
    response.body = public_wire::refusal("request_timeout");
    response
}

pub(crate) fn payload_too_large() -> RunnerResponse {
    RunnerResponse {
        status: 413,
        // Refused before any stage ran, but every answer carries the header all the same.
        headers: vec![(
            "server-timing",
            "auth;dur=0.000,pool;dur=0.000,exec;dur=0.000".to_owned(),
        )],
        body: public_wire::refusal("payload_too_large"),
        stream: None,
        journal: None,
    }
}
