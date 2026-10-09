//! `POST /api/v1/sandboxes/{id}/exec/stream`: a command answered as server-sent events (C7).
//!
//! The facade returns a command's output when the command ends, so the events carry it then:
//! `stdout`, `stderr`, and a final `exit` (or `error`). Until then the stream sends an SSE
//! comment every few seconds, so a long command keeps its connection and every proxy on the way
//! sees bytes moving. When the facade gains incremental output, the same events arrive as the
//! guest produces them, with no change to this contract.

use std::{sync::Arc, time::Duration, time::Instant};

use bytes::Bytes;
use serde::Serialize;
use tokio::sync::mpsc;

use crate::runner::{
    backend::{Backend, Busy},
    journal::Journal,
    keys::Principal,
    public_wire::PlatformError,
    sandboxes::Sandboxes,
};

use super::{
    Runner, RunnerResponse, Timing,
    command::{Prepared, command_entry},
    millis,
    outcome::{exit_code, failure_error},
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
    pub(super) fn exec_stream(
        &self,
        principal: &Principal,
        raw_id: &str,
        body: &[u8],
        received: Instant,
        timing: &Timing,
    ) -> RunnerResponse {
        let Prepared { id, owner, request } = match self.prepare_command(principal, raw_id, body) {
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
        let mut response = RunnerResponse::new(200, Vec::new())
            .header("content-type", "text/event-stream".to_owned())
            .header("server-timing", timing.header());
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
        let call = self.backend.call(move |facade| facade.execute(request));
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
                    self.event(
                        "stdout",
                        &Text {
                            text: &String::from_utf8_lossy(&executed.stdout),
                        },
                    )
                    .await;
                    self.event(
                        "stderr",
                        &Text {
                            text: &String::from_utf8_lossy(&executed.stderr),
                        },
                    )
                    .await;
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
