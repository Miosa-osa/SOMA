//! The response body every runner answer is sent as.

use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

use bytes::Bytes;
use hyper::body::{Body, Frame, SizeHint};

use crate::runner::journal::{Entry, Journal};

/// A response body that journals its request once the body is gone.
///
/// Hyper drops the body after its last frame is written, or when the connection fails first, so
/// the journal entry is written strictly after the response left this process: the paperwork is
/// never on the request path (contract C4).
///
/// A streamed answer (server-sent events, a forwarded response) sends its first bytes, then
/// every chunk its producer sends, and ends when the producer drops its sender.
pub struct JournaledBody {
    pub(super) head: Option<Bytes>,
    pub(super) stream: Option<tokio::sync::mpsc::Receiver<Bytes>>,
    pub(super) after: Option<(Journal, Entry)>,
    /// The producer's word that it sent everything (see `RunnerResponse::complete`).
    pub(super) complete: Option<Arc<AtomicBool>>,
}

impl Body for JournaledBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        if let Some(head) = self.head.take().filter(|head| !head.is_empty()) {
            return Poll::Ready(Some(Ok(Frame::data(head))));
        }
        let Some(stream) = self.stream.as_mut() else {
            return Poll::Ready(None);
        };
        match stream.poll_recv(context) {
            Poll::Ready(Some(bytes)) => Poll::Ready(Some(Ok(Frame::data(bytes)))),
            // A producer that stopped before its last byte: an error makes hyper abort the
            // response, a reset on HTTP/2 and a closed connection on HTTP/1.1, where ending it
            // would hand the client a short body that reads as whole.
            Poll::Ready(None) if cut_short(self.complete.as_ref()) => {
                self.complete = None;
                Poll::Ready(Some(Err(io::Error::other("the body was cut short"))))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
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

/// Whether a stream that just ended was cut short by its producer.
pub(crate) fn cut_short(complete: Option<&Arc<AtomicBool>>) -> bool {
    complete.is_some_and(|complete| !complete.load(Ordering::Acquire))
}
