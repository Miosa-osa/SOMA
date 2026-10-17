//! `POST /api/v1/sandboxes/{id}/exec/stream`: a command answered as server-sent events (C7).
//!
//! The events are `stdout` and `stderr`, each as the guest produces output, and a final `exit`
//! (or `error`). A backend that cannot observe output while the command runs delivers it when
//! the command ends, in the same events. Between events the stream sends an SSE comment every
//! few seconds, so a long command keeps its connection and every proxy on the way sees bytes
//! moving.
//!
//! An event carries text, so a chunk that ends inside a multi-byte character holds the partial
//! character back until the next chunk completes it ([`Utf8Splitter`]). A stream with no output
//! sends no `stdout` or `stderr` event at all; it used to send one empty event of each.

use std::{sync::Arc, time::Duration, time::Instant};

use bytes::Bytes;
use serde::Serialize;
use tokio::sync::mpsc;

use crate::runner::{
    backend::{Backend, Busy},
    journal::{EntryKind, Journal},
    keys::Principal,
    public_wire::PlatformError,
    sandboxes::Sandboxes,
};

mod split;

use split::Utf8Splitter;

use super::{
    Runner, RunnerResponse, Timing,
    command::{Prepared, command_entry},
    millis,
    outcome::{exit_code, failure_error},
    refused_sandbox, with_journal,
};

/// How often a running command's stream says it is still alive.
const KEEPALIVE: Duration = Duration::from_secs(10);
/// Events buffered for a slow reader before the producer waits.
const STREAM_BUFFER: usize = 16;

#[derive(Serialize)]
struct Text<'a> {
    text: &'a str,
}

#[derive(Serialize)]
struct Exit {
    exit_code: i32,
}

impl Runner {
    /// The stream is opened after a sandbox whose create answered early has been waited for, so
    /// the caller's events begin when there is a machine to produce them rather than with an
    /// error about one that is still coming up.
    pub(super) async fn exec_stream(
        &self,
        principal: &Principal,
        raw_id: &str,
        body: &[u8],
        received: Instant,
        timing: &mut Timing,
    ) -> RunnerResponse {
        let id = match self.addressed(raw_id) {
            Ok(id) => id,
            Err(response) => {
                return refused_sandbox(*response, EntryKind::Exec, principal);
            }
        };
        let waited = match self.wait_for_launch(&id, &principal.key.tenant_id).await {
            Ok(waited) => waited,
            Err(response) => {
                let status = response.status;
                return with_journal(
                    *response,
                    Some(command_entry(principal, &id, None, status, None)),
                );
            }
        };
        timing.wait = waited;
        let Prepared { id, owner, request } = match self.prepare_command(principal, &id, body) {
            Ok(prepared) => prepared,
            Err(response) => return *response,
        };
        let (sender, events) = mpsc::channel(STREAM_BUFFER);
        let run = Run {
            backend: self.backend.clone(),
            sandboxes: Arc::clone(&self.sandboxes),
            journal: self.journal.clone(),
            sender,
        };
        let principal = principal.clone();
        tokio::spawn(async move {
            let (status, exit_code) = run.command(&id, request).await;
            run.journal.record({
                let mut entry = command_entry(&principal, &id, Some(&owner), status, exit_code);
                entry.ms = millis(received.elapsed());
                entry
            });
        });
        let header = timing.header();
        let mut response = RunnerResponse::new(200, Vec::new())
            .header("content-type", "text/event-stream".to_owned())
            .header("server-timing", header);
        response.stream = Some(events);
        response
    }
}

struct Run {
    backend: Backend,
    sandboxes: Arc<Sandboxes>,
    journal: Journal,
    sender: mpsc::Sender<Bytes>,
}

impl Run {
    /// Runs the command, streaming keepalives and then its events; returns the status and exit
    /// code the journal records.
    async fn command(
        &self,
        id: &crate::runner::ids::SandboxId,
        request: soma::ExecuteMachineRequest,
    ) -> (u16, Option<i32>) {
        let events = self.sender.clone();
        let call = self.backend.call(move |facade| {
            let (mut stdout, mut stderr) = (Utf8Splitter::default(), Utf8Splitter::default());
            let mut reader_gone = false;
            let mut send = |name: &str, text: String| {
                if text.is_empty() || reader_gone {
                    return;
                }
                let data = crate::runner::public_wire::encode(&Text { text: &text });
                // The command runs on a blocking thread, so waiting for a slow reader here is
                // backpressure on the guest exchange, bounded by the command's own deadline.
                reader_gone = events.blocking_send(frame(name, &data)).is_err();
            };
            let result = facade.execute_streaming(request, &mut |channel, bytes| match channel {
                soma::OutputChannel::Stdout => send("stdout", stdout.push(bytes)),
                soma::OutputChannel::Stderr => send("stderr", stderr.push(bytes)),
            });
            send("stdout", stdout.finish());
            send("stderr", stderr.finish());
            result
        });
        tokio::pin!(call);
        let mut keepalive = tokio::time::interval(KEEPALIVE);
        let outcome = loop {
            tokio::select! {
                outcome = &mut call => break outcome,
                _tick = keepalive.tick() => {
                    // A reader that went away does not stop the command; its paperwork is
                    // still owed.
                    let _sent = self.sender.send(Bytes::from_static(b": running\n\n")).await;
                }
            }
        };
        self.sandboxes.release(id);
        match outcome {
            Ok((Ok(executed), _)) => {
                if let Some(code) = exit_code(executed.status) {
                    self.event("exit", &Exit { exit_code: code }).await;
                    (200, Some(code))
                } else {
                    self.error(&PlatformError::agent_unavailable()).await
                }
            }
            Ok((Err(failure), _)) => self.error(&failure_error(&failure)).await,
            Err(Busy) => {
                let busy = PlatformError::new(
                    429,
                    "RUNTIME_BUSY",
                    "the runner is at its admission cap; retry",
                    true,
                );
                self.error(&busy).await
            }
        }
    }

    async fn error(&self, error: &PlatformError) -> (u16, Option<i32>) {
        let body = error.body();
        let _sent = self.sender.send(frame("error", &body)).await;
        (error.status, None)
    }

    async fn event<T: Serialize>(&self, name: &str, data: &T) {
        let data = crate::runner::public_wire::encode(data);
        let _sent = self.sender.send(frame(name, &data)).await;
    }
}

/// One SSE event. JSON never holds a raw newline, so the data is always one `data:` line.
fn frame(name: &str, data: &[u8]) -> Bytes {
    let mut bytes = Vec::with_capacity(name.len() + data.len() + 16);
    bytes.extend_from_slice(b"event: ");
    bytes.extend_from_slice(name.as_bytes());
    bytes.extend_from_slice(b"\ndata: ");
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(b"\n\n");
    Bytes::from(bytes)
}

#[cfg(test)]
mod tests {
    use super::frame;

    #[test]
    fn an_event_is_one_data_line() {
        assert_eq!(
            &frame("exit", br#"{"exit_code":0}"#)[..],
            b"event: exit\ndata: {\"exit_code\":0}\n\n"
        );
    }
}
