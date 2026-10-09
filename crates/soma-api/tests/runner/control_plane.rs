//! A fake control plane on mTLS: the key feed and the journal ingest.

use std::{
    collections::BTreeMap,
    convert::Infallible,
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Frame, Incoming};
use hyper_util::rt::TokioIo;
use tokio::sync::mpsc;

use crate::support::{chain, key, provider, roots};

/// A streamed response body fed from a channel, so the test decides when each feed line goes.
pub(crate) struct ChannelBody(mpsc::UnboundedReceiver<Bytes>);

impl Body for ChannelBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        self.0
            .poll_recv(context)
            .map(|chunk| chunk.map(|bytes| Ok(Frame::data(bytes))))
    }
}

#[derive(Default)]
pub(crate) struct ControlPlaneState {
    /// Every open feed response; each runner holds one.
    pub(crate) feed: Mutex<Vec<mpsc::UnboundedSender<Bytes>>>,
    pub(crate) afters: Mutex<Vec<u64>>,
    pub(crate) journal: Mutex<BTreeMap<u64, serde_json::Value>>,
    pub(crate) posts: AtomicUsize,
}

impl ControlPlaneState {
    pub(crate) fn send(&self, line: &str) {
        // The count is taken and the lock released before the assertion, because a panic while
        // holding the mutex poisons it and every later reader then fails for that instead of for
        // what went wrong.
        let connected = {
            let mut feed = self.feed.lock().expect("feed lock");
            feed.retain(|sender| sender.send(Bytes::from(format!("{line}\n"))).is_ok());
            feed.len()
        };
        assert!(connected > 0, "a runner is connected to the feed");
    }

    /// Ends the current feed response, as a control-plane restart would.
    pub(crate) fn drop_feed(&self) {
        self.feed.lock().expect("feed lock").clear();
    }

    pub(crate) fn connections(&self) -> Vec<u64> {
        self.afters.lock().expect("afters lock").clone()
    }

    pub(crate) fn journal(&self) -> BTreeMap<u64, serde_json::Value> {
        self.journal.lock().expect("journal lock").clone()
    }
}

pub(crate) type ControlPlaneBody = http_body_util::Either<ChannelBody, Full<Bytes>>;

pub(crate) async fn control_plane_answer(
    state: Arc<ControlPlaneState>,
    request: http::Request<Incoming>,
) -> http::Response<ControlPlaneBody> {
    let path = request.uri().path().to_owned();
    if path == "/internal/soma-runner/feed" {
        let after = request
            .uri()
            .query()
            .and_then(|query| query.strip_prefix("after="))
            .and_then(|after| after.parse().ok())
            .expect("the runner always sends after=");
        // The sender is registered before the `after` mark, so a test that waits for the mark and
        // then sends into the feed cannot find it empty: the two are recorded separately, and the
        // other order let a test see two connections and still have nothing to send to.
        let (sender, receiver) = mpsc::unbounded_channel();
        state.feed.lock().expect("feed lock").push(sender);
        state.afters.lock().expect("afters lock").push(after);
        return http::Response::new(http_body_util::Either::Left(ChannelBody(receiver)));
    }
    assert_eq!(path, "/internal/soma-runner/journal");
    assert_eq!(
        request.headers()[http::header::CONTENT_TYPE],
        "application/x-ndjson"
    );
    // A runner whose connection drops mid-post is a runner that will post again, so a body that
    // cannot be read answers as though nothing had been acknowledged rather than taking the whole
    // test down with it.
    let Ok(body) = request.into_body().collect().await else {
        return http::Response::new(http_body_util::Either::Right(Full::new(
            Bytes::from_static(b"{\"acked\":0}"),
        )));
    };
    let body = body.to_bytes();
    state.posts.fetch_add(1, Ordering::SeqCst);
    let mut journal = state.journal.lock().expect("journal lock");
    for line in body
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let value: serde_json::Value = serde_json::from_slice(line).expect("a JSON line");
        let offset = value["offset"].as_u64().expect("offset");
        journal.insert(offset, value);
    }
    let mut acked = 0;
    while journal.contains_key(&(acked + 1)) {
        acked += 1;
    }
    http::Response::new(http_body_util::Either::Right(Full::new(Bytes::from(
        format!(r#"{{"acked":{acked}}}"#),
    ))))
}

/// Starts an mTLS control plane that only accepts clients issued by the test CA.
pub(crate) async fn start_control_plane() -> (SocketAddr, Arc<ControlPlaneState>) {
    let verifier =
        rustls::server::WebPkiClientVerifier::builder_with_provider(Arc::new(roots()), provider())
            .build()
            .expect("client verifier");
    let mut tls = rustls::ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_client_cert_verifier(verifier)
        .with_single_cert(chain("server.pem"), key("server-key.pem"))
        .expect("server cert");
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("address");
    let state = Arc::new(ControlPlaneState::default());
    let served = Arc::clone(&state);
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.expect("accept");
            let (acceptor, state) = (acceptor.clone(), Arc::clone(&served));
            tokio::spawn(async move {
                let Ok(stream) = acceptor.accept(stream).await else {
                    return;
                };
                let service = hyper::service::service_fn(move |request| {
                    let state = Arc::clone(&state);
                    async move { Ok::<_, Infallible>(control_plane_answer(state, request).await) }
                });
                let _closed = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    (address, state)
}
