use std::{io, sync::Arc, time::Duration};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1::SendRequest;
use serde::Deserialize;

use crate::runner::{
    control_plane::Client,
    journal::Journal,
    journal_reader::{AckFile, Reader, ShippedLine},
};

const JOURNAL_PATH: &str = "/internal/soma-runner/journal";
/// How long the shipper sleeps when there is nothing new, if no append wakes it first.
const IDLE_POLL: Duration = Duration::from_secs(1);
const MIN_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(10);
const MAX_ACK_BYTES: usize = 4_096;

#[derive(Deserialize)]
struct Ack {
    acked: u64,
}

/// Where the reader goes after the control plane acknowledges a batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resume {
    /// The whole batch is stored; keep reading from where the batch ended.
    Continue,
    /// Part of the batch is stored; resend from this byte position.
    SeekTo(u64),
    /// The acknowledgement falls outside the batch; find `acked + 1` from the start.
    Rescan,
}

/// Decides how shipping resumes after `acked` comes back for `batch`.
#[must_use]
pub fn resume(batch: &[ShippedLine], acked: u64) -> Resume {
    let (Some(first), Some(last)) = (batch.first(), batch.last()) else {
        return Resume::Rescan;
    };
    if acked == last.offset {
        return Resume::Continue;
    }
    if acked + 1 < first.offset || acked > last.offset {
        return Resume::Rescan;
    }
    batch
        .iter()
        .find(|line| line.offset == acked + 1)
        .map_or(Resume::Rescan, |line| Resume::SeekTo(line.position))
}

/// Ships the journal to the control plane for the life of the process.
///
/// Each batch is posted as NDJSON; the control plane answers with the highest contiguous offset
/// it has stored, which is persisted beside the journal so a restarted shipper resumes from
/// `acked + 1` (contract C4). Ingest is idempotent on `(runner, offset)`, so a resend after a
/// lost answer costs nothing but the bytes.
pub fn spawn(client: Arc<Client>, journal: Journal, batch_lines: usize) {
    tokio::spawn(async move {
        let mut shipper = Shipper {
            acks: AckFile::beside(journal.path()),
            client,
            journal,
            batch_lines,
            reader: None,
            sender: None,
            acked: 0,
        };
        shipper.run().await;
    });
}

struct Shipper {
    client: Arc<Client>,
    journal: Journal,
    acks: AckFile,
    batch_lines: usize,
    reader: Option<Reader>,
    sender: Option<SendRequest<Full<Bytes>>>,
    acked: u64,
}

impl Shipper {
    async fn run(&mut self) {
        let mut backoff = MIN_BACKOFF;
        loop {
            match self.acks.load() {
                Ok(acked) => {
                    self.acked = acked;
                    break;
                }
                Err(error) => {
                    eprintln!("soma-api: runner shipper: {error}");
                    tokio::time::sleep(MAX_BACKOFF).await;
                }
            }
        }
        loop {
            if self.journal.durable_offset() <= self.acked {
                let _woken = tokio::time::timeout(IDLE_POLL, self.journal.appended()).await;
                continue;
            }
            match self.ship_batch().await {
                Ok(()) => backoff = MIN_BACKOFF,
                Err(error) => {
                    eprintln!("soma-api: runner shipper: {error}");
                    self.sender = None;
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            }
        }
    }

    async fn ship_batch(&mut self) -> io::Result<()> {
        let path = self.journal.path().to_owned();
        let acked = self.acked;
        let mut reader = match self.reader.take() {
            Some(reader) => reader,
            None => blocking(move || Reader::after(&path, acked)).await?,
        };
        let (batch_lines, durable) = (self.batch_lines, self.journal.durable_offset());
        let (reader, batch) = blocking(move || {
            let batch = reader.batch(batch_lines, durable)?;
            Ok((reader, batch))
        })
        .await?;
        if batch.is_empty() {
            // Durable is ahead of the acknowledgement but nothing was readable from here; the
            // position is stale, so the next round finds `acked + 1` from the start.
            return Ok(());
        }
        self.reader = Some(reader);
        let stored = match self.post(&batch).await {
            Ok(stored) => stored,
            Err(error) => {
                // Resend this same batch next time without rescanning the journal.
                if let (Some(reader), Some(first)) = (self.reader.as_mut(), batch.first()) {
                    reader.seek_to(first.position);
                }
                return Err(error);
            }
        };
        match resume(&batch, stored) {
            Resume::Continue => {}
            Resume::SeekTo(position) => {
                if let Some(reader) = self.reader.as_mut() {
                    reader.seek_to(position);
                }
            }
            Resume::Rescan => self.reader = None,
        }
        if stored != self.acked {
            self.acks.store(stored)?;
            self.acked = stored;
        }
        Ok(())
    }

    async fn post(&mut self, batch: &[ShippedLine]) -> io::Result<u64> {
        let mut body = Vec::with_capacity(batch.iter().map(|line| line.bytes.len()).sum());
        for line in batch {
            body.extend_from_slice(&line.bytes);
        }
        let request = self.client.request(
            http::Method::POST,
            JOURNAL_PATH,
            Some("application/x-ndjson"),
            Bytes::from(body),
        )?;
        let sender = match self.sender.as_mut() {
            Some(sender) if !sender.is_closed() => sender,
            _ => self.sender.insert(self.client.connect().await?),
        };
        let response = sender
            .send_request(request)
            .await
            .map_err(io::Error::other)?;
        let status = response.status();
        let bytes = http_body_util::Limited::new(response.into_body(), MAX_ACK_BYTES)
            .collect()
            .await
            .map_err(io::Error::other)?
            .to_bytes();
        if status != http::StatusCode::OK {
            return Err(io::Error::other(format!(
                "the control plane answered {status}"
            )));
        }
        let ack: Ack = serde_json::from_slice(&bytes)
            .map_err(|_| io::Error::other("the acknowledgement is not {\"acked\":N}"))?;
        Ok(ack.acked)
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(io::Error::other)?
}

#[cfg(test)]
mod tests {
    use super::{Resume, resume};
    use crate::runner::journal_reader::ShippedLine;

    fn batch(offsets: std::ops::RangeInclusive<u64>) -> Vec<ShippedLine> {
        offsets
            .map(|offset| ShippedLine {
                offset,
                position: offset * 100,
                bytes: Vec::new(),
            })
            .collect()
    }

    #[test]
    fn a_whole_acknowledgement_continues() {
        assert_eq!(resume(&batch(5..=9), 9), Resume::Continue);
    }

    #[test]
    fn a_partial_acknowledgement_resends_from_the_next_offset() {
        assert_eq!(resume(&batch(5..=9), 6), Resume::SeekTo(700));
        assert_eq!(resume(&batch(5..=9), 4), Resume::SeekTo(500));
    }

    #[test]
    fn an_acknowledgement_outside_the_batch_rescans() {
        assert_eq!(resume(&batch(5..=9), 2), Resume::Rescan);
        assert_eq!(resume(&batch(5..=9), 12), Resume::Rescan);
        assert_eq!(resume(&[], 3), Resume::Rescan);
    }
}
