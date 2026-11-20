//! One request and one answer as the runner sees them, independent of the HTTP version that
//! carried them, and the answers every route shares.

use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

use bytes::Bytes;

use crate::runner::{
    body::StreamedBody,
    journal::Entry,
    public_wire::{self, PlatformError},
};

use super::TunnelHandoff;

/// One request, after the transport has read it, independent of HTTP version.
#[derive(Clone, Debug)]
pub struct RunnerRequest {
    pub method: http::Method,
    pub path: String,
    /// The query string, without its `?`, when the request carried one.
    pub query: Option<String>,
    pub authorization: Option<String>,
    pub body: Bytes,
    pub received: Instant,
    /// Arrived from another runner over the private listener: never forwarded again.
    pub forwarded: bool,
    /// An HTTP/1.1 request that asked for `Connection: Upgrade` and `Upgrade: soma-tunnel`.
    pub upgrade: bool,
    /// The `Content-Type` header, which an upload's multipart boundary is read from.
    pub content_type: Option<String>,
    /// The body, handed on as it arrives, for the one route that streams it; `body` is then
    /// empty.
    pub streamed: Option<StreamedBody>,
}

/// One answer, plus the journal entry to write once it has been sent.
#[derive(Debug)]
pub struct RunnerResponse {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
    /// A body that is still being produced (server-sent events, a forwarded answer); sent
    /// after `body`, chunk by chunk, until the sender is dropped.
    pub stream: Option<tokio::sync::mpsc::Receiver<Bytes>>,
    pub journal: Option<Entry>,
    /// A guest connection to pump after the `101` is sent; the transport owns the upgrade.
    pub tunnel: Option<TunnelHandoff>,
    /// Set by a stream's producer once it has sent its last byte. A stream that ends while this
    /// is present and unset was cut short, and the transport aborts it instead of ending it, so a
    /// client never reads part of a file as the whole of it.
    pub complete: Option<Arc<AtomicBool>>,
}

impl RunnerResponse {
    pub(super) fn new(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body,
            stream: None,
            journal: None,
            tunnel: None,
            complete: None,
        }
    }

    pub(super) fn platform(error: &PlatformError) -> Self {
        Self::new(error.status, error.body())
    }

    pub(super) fn refusal(status: u16, code: &str) -> Self {
        Self::new(status, public_wire::refusal(code))
    }

    /// The `503` that tells the SDK to retry on the next runner at once.
    /// `429 runtime_busy`: this runner is at its admission cap; the SDK retries the next IP.
    pub(super) fn runtime_busy() -> Self {
        Self::refusal(429, "runtime_busy").header("retry-after", "0".to_owned())
    }

    /// `503 feed_stale`: this runner's key table is too old to admit a create.
    pub(super) fn feed_stale() -> Self {
        Self::refusal(503, "feed_stale").header("retry-after", "0".to_owned())
    }

    /// `503`: this runner could not claim a prepared sandbox.
    ///
    /// One constructor for the whole service, because a create's own failure and the same
    /// failure read later by a call on a sandbox whose launch failed after its create had
    /// answered must be the same answer, byte for byte.
    pub(super) fn create_unavailable() -> Self {
        Self::new(503, public_wire::create_unavailable()).header("retry-after", "1".to_owned())
    }

    pub(super) fn header(mut self, name: &'static str, value: String) -> Self {
        self.headers.push((name, value));
        self
    }
}
