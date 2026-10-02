use std::{io, path::PathBuf, sync::Arc, time::Duration};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1::SendRequest;
use serde::Deserialize;

use crate::runner::{
    control_plane::Client,
    journal::Journal,
    journal_reader::{AckFile, Reader, ShippedLine, epoch_files},
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
/// Boot epochs ship oldest first. Each batch holds lines of one epoch and is posted as NDJSON;
/// the control plane answers with the highest contiguous offset it stored for that epoch, which
/// is persisted beside the epoch's file so a restarted shipper resumes from `acked + 1`
/// (contract C4). An earlier epoch that is fully delivered is deleted. Ingest is idempotent on
/// `(runner, boot_epoch, offset)`, so a resend after a lost answer costs nothing but the bytes.
pub fn spawn(client: Arc<Client>, journal: Journal, batch_lines: usize) {
    tokio::spawn(async move {
        let mut shipper = Shipper {
            client,
            journal,
            batch_lines,
            target: None,
            sender: None,
        };
        shipper.run().await;
    });
}

struct Shipper {
    client: Arc<Client>,
    journal: Journal,
    batch_lines: usize,
    target: Option<Target>,
    sender: Option<SendRequest<Full<Bytes>>>,
}

/// The epoch file being shipped, with its reader and acknowledgement.
struct Target {
    epoch: String,
    path: PathBuf,
    acks: AckFile,
    acked: u64,
    reader: Reader,
}

impl Target {
    fn open(epoch: String, path: PathBuf) -> io::Result<Self> {
        let acks = AckFile::beside(&path);
        let acked = acks.load()?;
        let reader = Reader::after(&path, acked)?;
        Ok(Self {
            epoch,
            path,
            acks,
            acked,
            reader,
        })
    }
}

/// Where the shipper reads: the epoch, its file, and how far its lines are written.
struct Source {
    directory: PathBuf,
    runner: String,
    current_epoch: String,
    written: u64,
    batch_lines: usize,
}

/// Finds the next batch to ship, oldest epoch first, deleting earlier epochs once delivered.
fn next_batch(
    mut target: Option<Target>,
    source: &Source,
) -> io::Result<(Option<Target>, Vec<ShippedLine>)> {
    loop {
        let mut current = match target.take() {
            Some(current) => current,
            None => match epoch_files(&source.directory, &source.runner)?
                .into_iter()
                .next()
            {
                Some((epoch, path)) => Target::open(epoch, path)?,
                None => return Ok((None, Vec::new())),
            },
        };
        let live = current.epoch == source.current_epoch;
        let written = if live { source.written } else { u64::MAX };
        let batch = current.reader.batch(source.batch_lines, written)?;
        if !batch.is_empty() || live {
            return Ok((Some(current), batch));
        }
        // An earlier epoch with nothing left past its acknowledgement is delivered.
        std::fs::remove_file(&current.path)?;
        match std::fs::remove_file(current.acks.path()) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
}

impl Shipper {
    async fn run(&mut self) {
        let mut backoff = MIN_BACKOFF;
        loop {
            match self.ship_batch().await {
                Ok(true) => backoff = MIN_BACKOFF,
                Ok(false) => {
                    let _woken = tokio::time::timeout(IDLE_POLL, self.journal.appended()).await;
                }
                Err(error) => {
                    eprintln!("soma-api: runner shipper: {error}");
                    self.sender = None;
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            }
        }
    }

    /// Ships one batch; returns whether there was anything to ship.
    async fn ship_batch(&mut self) -> io::Result<bool> {
        let source = Source {
            directory: self.journal.directory().to_owned(),
            runner: self.journal.runner().to_owned(),
            current_epoch: self.journal.epoch().to_owned(),
            written: self.journal.written_offset(),
            batch_lines: self.batch_lines,
        };
        let target = self.target.take();
        let (target, batch) = blocking(move || next_batch(target, &source)).await?;
        self.target = target;
        if batch.is_empty() {
            return Ok(false);
        }
        let stored = match self.post(&batch).await {
            Ok(stored) => stored,
            Err(error) => {
                // Resend this same batch next time without rescanning the journal.
                if let (Some(target), Some(first)) = (self.target.as_mut(), batch.first()) {
                    target.reader.seek_to(first.position);
                }
                return Err(error);
            }
        };
        let Some(target) = self.target.as_mut() else {
            return Ok(true);
        };
        match resume(&batch, stored) {
            Resume::Continue => {}
            Resume::SeekTo(position) => target.reader.seek_to(position),
            Resume::Rescan => target.reader.seek_after(stored)?,
        }
        if stored != target.acked {
            target.acks.store(stored)?;
            target.acked = stored;
        }
        Ok(true)
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
#[path = "shipper_tests.rs"]
mod tests;
