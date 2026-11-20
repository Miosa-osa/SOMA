//! One request and one answer as the runner sees them, independent of the HTTP version that
//! carried them; the answers every route shares are built in `response`.

use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

use bytes::Bytes;

use crate::runner::{body::StreamedBody, journal::Entry};

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
