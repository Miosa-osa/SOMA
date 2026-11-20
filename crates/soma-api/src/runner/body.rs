//! A request body handed to the runner as it arrives rather than after it has all arrived.
//!
//! Every other route's body is read whole before the runner sees the request, under a bound
//! small enough to hold. A file upload is the one request whose body may be larger than anything
//! this process should hold, so its transport reads the body on a task of its own and hands each
//! chunk over a bounded channel: the reader takes a chunk when it is ready for one, and the
//! client is held back by the transport's own flow control until then.
//!
//! The transport still bounds what it will carry, in total and in time: a body larger than the
//! route accepts, or one that stops moving for as long as a whole body is allowed to take to
//! arrive elsewhere, ends with a fault rather than a short body that would read as complete.
//!
//! The transport does not touch the body until the route takes it. A request the runner refuses
//! first (another runner's sandbox, a sandbox that does not exist, a malformed form) is answered
//! without a byte of its body read, so a client that sent `Expect: 100-continue` is never told
//! to send a file nobody will read, and a proxy in front can still send the unread body
//! somewhere else.

use std::sync::{Arc, Mutex};

use bytes::{Buf, Bytes};
use http_body_util::BodyExt;
use tokio::sync::{mpsc, oneshot};

use crate::runner::limits::BODY_READ_TIMEOUT;

/// Chunks buffered between the transport and the reader.
const CHUNKS_IN_FLIGHT: usize = 8;

/// Why a streamed body ended before its last byte.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BodyFault {
    /// The body is larger than the route accepts.
    TooLarge,
    /// No byte arrived for longer than a body may stall.
    Stalled,
    /// The connection failed or the client abandoned the request.
    Broken,
}

/// One chunk of a streamed body, or the fault that ended it.
pub type BodyChunk = Result<Bytes, BodyFault>;

/// The receiving end of a streamed body; taken once, by the route that reads it.
#[derive(Clone, Default)]
pub struct StreamedBody(Arc<Mutex<Option<Taken>>>);

type Taken = (mpsc::Receiver<BodyChunk>, oneshot::Sender<()>);

/// The transport's end of a streamed body: a sender it may use once the route takes the body.
#[derive(Debug)]
pub struct Feeder {
    sender: mpsc::Sender<BodyChunk>,
    taken: oneshot::Receiver<()>,
}

impl Feeder {
    /// Waits until the route takes the body; `None` when the request ended without taking it.
    pub async fn wait(self) -> Option<mpsc::Sender<BodyChunk>> {
        self.taken.await.ok().map(|()| self.sender)
    }
}

impl StreamedBody {
    /// A body and the feeder its transport fills it from.
    #[must_use]
    pub fn channel() -> (Feeder, Self) {
        let (sender, receiver) = mpsc::channel(CHUNKS_IN_FLIGHT);
        let (start, taken) = oneshot::channel();
        (
            Feeder { sender, taken },
            Self(Arc::new(Mutex::new(Some((receiver, start))))),
        )
    }

    /// The chunks, for the one reader that takes them; taking them starts the transport.
    #[must_use]
    pub fn take(&self) -> Option<mpsc::Receiver<BodyChunk>> {
        let (receiver, start) = self.0.lock().ok()?.take()?;
        // The transport may already have gone, with the request; then nothing will arrive and
        // the reader sees the channel end.
        let _started = start.send(());
        Some(receiver)
    }
}

impl std::fmt::Debug for StreamedBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("StreamedBody")
    }
}

/// Feeds an HTTP/1.1 or HTTP/2 body to its reader, once taken, until it ends, faults, or its
/// reader leaves.
pub async fn pump_hyper(mut body: hyper::body::Incoming, feeder: Feeder, limit: u64) {
    let Some(sender) = feeder.wait().await else {
        return;
    };
    let mut carried = 0_u64;
    loop {
        let chunk = match tokio::time::timeout(BODY_READ_TIMEOUT, body.frame()).await {
            Ok(Some(Ok(frame))) => match frame.into_data() {
                Ok(data) => Ok(data),
                // Trailers carry no body bytes.
                Err(_trailers) => continue,
            },
            Ok(None) => return,
            Ok(Some(Err(_))) => Err(BodyFault::Broken),
            Err(_) => Err(BodyFault::Stalled),
        };
        if !forward(&sender, chunk, &mut carried, limit).await {
            return;
        }
    }
}

/// Feeds an HTTP/3 request body into `sender`, as [`pump_hyper`] does.
pub async fn pump_h3(
    mut stream: h3::server::RequestStream<h3_quinn::RecvStream, Bytes>,
    feeder: Feeder,
    limit: u64,
) {
    let Some(sender) = feeder.wait().await else {
        return;
    };
    let mut carried = 0_u64;
    loop {
        let chunk = match tokio::time::timeout(BODY_READ_TIMEOUT, stream.recv_data()).await {
            Ok(Ok(Some(mut data))) => Ok(data.copy_to_bytes(data.remaining())),
            Ok(Ok(None)) => return,
            Ok(Err(_)) => Err(BodyFault::Broken),
            Err(_) => Err(BodyFault::Stalled),
        };
        if !forward(&sender, chunk, &mut carried, limit).await {
            return;
        }
    }
}

/// Hands one chunk on; `false` once the body is over for either side.
async fn forward(
    sender: &mpsc::Sender<BodyChunk>,
    chunk: BodyChunk,
    carried: &mut u64,
    limit: u64,
) -> bool {
    let chunk = chunk.and_then(|data| {
        *carried = carried.saturating_add(u64::try_from(data.len()).unwrap_or(u64::MAX));
        if *carried > limit {
            Err(BodyFault::TooLarge)
        } else {
            Ok(data)
        }
    });
    let ended = chunk.is_err();
    sender.send(chunk).await.is_ok() && !ended
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::{BodyFault, StreamedBody, forward};

    #[tokio::test]
    async fn a_body_past_its_limit_ends_with_a_fault_rather_than_a_short_body() {
        let (feeder, body) = StreamedBody::channel();
        let mut receiver = body.take().expect("the first reader takes it");
        assert!(body.take().is_none(), "and only the first");
        let sender = feeder.wait().await.expect("taken");
        let mut carried = 0;
        assert!(forward(&sender, Ok(Bytes::from_static(b"abc")), &mut carried, 4).await);
        assert!(!forward(&sender, Ok(Bytes::from_static(b"de")), &mut carried, 4).await);
        assert_eq!(receiver.recv().await, Some(Ok(Bytes::from_static(b"abc"))));
        assert_eq!(receiver.recv().await, Some(Err(BodyFault::TooLarge)));
    }

    #[tokio::test]
    async fn a_reader_that_left_stops_the_transport() {
        let (feeder, body) = StreamedBody::channel();
        drop(body.take());
        let sender = feeder.wait().await.expect("taken");
        let mut carried = 0;
        assert!(!forward(&sender, Ok(Bytes::from_static(b"a")), &mut carried, 10).await);
    }

    #[tokio::test]
    async fn a_body_nobody_takes_is_never_read() {
        let (feeder, body) = StreamedBody::channel();
        drop(body);
        assert!(feeder.wait().await.is_none());
    }
}
