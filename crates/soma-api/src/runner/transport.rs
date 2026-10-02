use std::{
    convert::Infallible,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Body, Frame, Incoming, SizeHint};
use hyper_util::{
    rt::{TokioExecutor, TokioIo, TokioTimer},
    server::conn::auto,
};

use crate::runner::{
    journal::{Entry, Journal},
    public_wire,
    service::{MAX_BODY_BYTES, Runner, RunnerRequest, RunnerResponse},
    tls::CertificateStore,
};

const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const MAX_STREAMS_PER_CONNECTION: u32 = 256;
const CONTENT_TYPE: &str = "application/json; charset=utf-8";

/// A response body that journals its request once the body is gone.
///
/// Hyper drops the body after its last frame is written, or when the connection fails first, so
/// the journal entry is written strictly after the response left this process: the paperwork is
/// never on the request path (contract C4).
pub struct JournaledBody {
    inner: Full<Bytes>,
    after: Option<(Journal, Entry)>,
}

impl Body for JournaledBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.inner).poll_frame(context)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
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
        let (stream, _peer) = match listener.accept().await {
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
        let (tls, runner) = (tls.clone(), Arc::clone(&runner));
        tokio::spawn(async move {
            let _permit = permit;
            if stream.set_nodelay(true).is_err() {
                return;
            }
            let Ok(Ok(stream)) =
                tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, tls.accept(stream)).await
            else {
                return;
            };
            serve_http(TokioIo::new(stream), runner).await;
        });
    }
}

/// Serves HTTP/1.1 or HTTP/2 on one established connection.
pub async fn serve_http<I>(io: I, runner: Arc<Runner>)
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let service = hyper::service::service_fn(move |request: http::Request<Incoming>| {
        let runner = Arc::clone(&runner);
        async move { Ok::<_, Infallible>(respond(&runner, request).await) }
    });
    let mut builder = auto::Builder::new(TokioExecutor::new());
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_READ_TIMEOUT);
    builder
        .http2()
        .timer(TokioTimer::new())
        .max_concurrent_streams(MAX_STREAMS_PER_CONNECTION);
    let _closed = builder.serve_connection(io, service).await;
}

async fn respond(
    runner: &Runner,
    request: http::Request<Incoming>,
) -> http::Response<JournaledBody> {
    let received = Instant::now();
    let (parts, body) = request.into_parts();
    let Ok(collected) = Limited::new(body, MAX_BODY_BYTES).collect().await else {
        return encode_hyper(payload_too_large(), None);
    };
    let response = runner
        .handle(RunnerRequest {
            method: parts.method,
            path: parts.uri.path().to_owned(),
            authorization: authorization(&parts.headers),
            body: collected.to_bytes(),
            received,
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
        inner: Full::new(Bytes::from(std::mem::take(&mut response.body))),
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

pub(crate) fn payload_too_large() -> RunnerResponse {
    RunnerResponse {
        status: 413,
        // Refused before any stage ran, but every answer carries the header all the same.
        headers: vec![(
            "server-timing",
            "auth;dur=0.000,pool;dur=0.000,exec;dur=0.000".to_owned(),
        )],
        body: public_wire::refusal("payload_too_large"),
        journal: None,
    }
}
