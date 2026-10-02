use std::{
    io,
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use http_body_util::BodyExt;

use crate::runner::{control_plane::Client, feed::FeedEvent, keys::KeyTable};

/// The control plane sends a heartbeat every 5 s; three missed ones end the connection.
const IDLE_LIMIT: Duration = Duration::from_secs(15);
const MIN_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(5);
/// No legitimate event is anywhere near this long; a line past it is a broken stream.
const MAX_LINE_BYTES: usize = 64 * 1024;
const FEED_PATH: &str = "/internal/soma-runner/feed";

/// Splits a growing byte buffer into complete feed lines and applies each to the table.
///
/// It is separate from the connection so the framing can be proved without a socket: chunk
/// boundaries from the network fall anywhere, including inside a line.
#[derive(Debug, Default)]
pub struct LineReader {
    buffer: Vec<u8>,
}

impl LineReader {
    /// Feeds one received chunk, applying every complete line it finishes.
    ///
    /// Returns how many events were applied.
    ///
    /// # Errors
    ///
    /// Returns an invalid-data error for an unparseable line, an event the table refuses, or a
    /// line longer than any real event. The caller reconnects from the last applied sequence.
    pub fn push(&mut self, chunk: &[u8], table: &KeyTable, now: Instant) -> io::Result<usize> {
        self.buffer.extend_from_slice(chunk);
        let mut applied = 0;
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = std::str::from_utf8(&line[..end])
                .map_err(|_| invalid("a feed line is not UTF-8"))?
                .trim();
            if line.is_empty() {
                continue;
            }
            let event = FeedEvent::parse(line).map_err(|error| invalid(&error.to_string()))?;
            table
                .apply(&event, now)
                .map_err(|violation| invalid(&format!("{violation:?}")))?;
            applied += 1;
        }
        if self.buffer.len() > MAX_LINE_BYTES {
            return Err(invalid("a feed line exceeded the line limit"));
        }
        Ok(applied)
    }
}

/// Follows the feed for the life of the process, reconnecting with `after=<last seq>`.
pub fn spawn(client: Arc<Client>, table: Arc<KeyTable>) {
    tokio::spawn(async move {
        let mut backoff = MIN_BACKOFF;
        loop {
            match follow(&client, &table).await {
                Ok(applied) if applied > 0 => backoff = MIN_BACKOFF,
                Ok(_) => {}
                Err(error) => eprintln!("soma-api: runner feed: {error}"),
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    });
}

/// Holds one feed connection open until it ends, returning how many events it applied.
async fn follow(client: &Client, table: &KeyTable) -> io::Result<usize> {
    table.begin_connection();
    let mut sender = client.connect().await?;
    let path = format!("{FEED_PATH}?after={}", table.last_seq());
    let request = client.request(http::Method::GET, &path, None, Bytes::new())?;
    let response = sender
        .send_request(request)
        .await
        .map_err(io::Error::other)?;
    if response.status() != http::StatusCode::OK {
        return Err(invalid(&format!(
            "the control plane answered {}",
            response.status()
        )));
    }
    let mut body = response.into_body();
    let mut reader = LineReader::default();
    let mut applied = 0;
    loop {
        let frame = tokio::time::timeout(IDLE_LIMIT, body.frame())
            .await
            .map_err(|_| invalid("no feed event within the idle limit"))?;
        match frame {
            None => return Ok(applied),
            Some(Err(error)) => return Err(io::Error::other(error)),
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    applied += reader.push(&data, table, Instant::now())?;
                }
            }
        }
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("feed: {message}"))
}
