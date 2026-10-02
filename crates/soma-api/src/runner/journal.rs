use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, SystemTime},
};

use serde::{Deserialize, Serialize};

use crate::runner::clock::rfc3339_millis;

/// How far back from the end of the file recovery looks for the last complete line.
const RECOVERY_WINDOW: u64 = 64 * 1024;
/// How many queued entries one write and one `fsync` cover at most.
const MAX_WRITE_BATCH: usize = 4_096;
const WRITE_RETRY: Duration = Duration::from_secs(1);

/// What one journal line records (contract C4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Create,
    Exec,
    Destroy,
}

/// One served request, as the runner knows it when the response leaves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub kind: EntryKind,
    pub tenant_id: String,
    pub key_id: Option<String>,
    pub project_id: Option<String>,
    pub sandbox_id: Option<String>,
    pub status: u16,
    pub ms: u64,
    pub exit_code: Option<i32>,
    pub cpu_ms: Option<u64>,
    pub lifetime_ms: Option<u64>,
}

/// One journal line in contract field order.
#[derive(Serialize)]
struct Line<'a> {
    runner: &'a str,
    offset: u64,
    ts: String,
    #[serde(flatten)]
    entry: &'a Entry,
}

#[derive(Deserialize)]
pub(crate) struct OffsetOnly {
    pub(crate) offset: u64,
}

/// The append-only paperwork journal of one runner.
///
/// Entries are queued without blocking and written by one thread, which assigns each the next
/// offset, appends the batch, and syncs it before publishing the new durable offset. Offsets
/// continue from the last line on disk across restarts, so an offset is never reused; a line
/// torn by a crash is cut off at startup rather than glued to the next one.
#[derive(Clone)]
pub struct Journal {
    sender: mpsc::Sender<Entry>,
    shared: Arc<Shared>,
}

struct Shared {
    path: PathBuf,
    runner: String,
    /// The highest offset that is written and synced.
    durable: AtomicU64,
    appended: tokio::sync::Notify,
}

impl Journal {
    /// Opens `<directory>/<runner>.ndjson`, recovering its last offset, and starts the writer.
    ///
    /// # Errors
    ///
    /// Returns the failure to create the directory, open the file, or recover its last line.
    pub fn open(directory: &Path, runner: &str) -> io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        let path = directory.join(format!("{runner}.ndjson"));
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&path)?;
        let last = recover(&mut file)?;
        let shared = Arc::new(Shared {
            path,
            runner: runner.to_owned(),
            durable: AtomicU64::new(last),
            appended: tokio::sync::Notify::new(),
        });
        let (sender, receiver) = mpsc::channel();
        let writer = Arc::clone(&shared);
        thread::Builder::new()
            .name("soma-runner-journal".to_owned())
            .spawn(move || write_loop(&writer, file, last, &receiver))?;
        Ok(Self { sender, shared })
    }

    /// Queues one entry. It never blocks the caller.
    pub fn record(&self, entry: Entry) {
        // The writer thread only stops when every sender is gone, so a send cannot fail while
        // this handle exists.
        let _queued = self.sender.send(entry);
    }

    /// The highest offset that is on disk and synced.
    #[must_use]
    pub fn durable_offset(&self) -> u64 {
        self.shared.durable.load(Ordering::Acquire)
    }

    /// Waits until the writer publishes a new durable offset.
    pub async fn appended(&self) {
        self.shared.appended.notified().await;
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.shared.path
    }

    #[must_use]
    pub fn runner(&self) -> &str {
        &self.shared.runner
    }
}

/// Cuts a torn last line, if any, and returns the offset of the last complete line.
fn recover(file: &mut File) -> io::Result<u64> {
    let length = file.metadata()?.len();
    if length == 0 {
        return Ok(0);
    }
    let window = length.min(RECOVERY_WINDOW);
    let start = length - window;
    file.seek(SeekFrom::Start(start))?;
    let mut tail = Vec::new();
    file.take(window).read_to_end(&mut tail)?;
    let Some(last_newline) = tail.iter().rposition(|byte| *byte == b'\n') else {
        if start == 0 {
            // A file holding only a torn first line: nothing in it was ever complete.
            file.set_len(0)?;
            return Ok(0);
        }
        return Err(corrupt("no complete line in the journal tail"));
    };
    let complete = start + last_newline as u64 + 1;
    if complete < length {
        file.set_len(complete)?;
    }
    let line_start = tail[..last_newline]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |position| position + 1);
    if line_start == 0 && start != 0 {
        return Err(corrupt("the last journal line exceeds the recovery window"));
    }
    let last: OffsetOnly = serde_json::from_slice(&tail[line_start..last_newline])
        .map_err(|_| corrupt("the last journal line carries no offset"))?;
    Ok(last.offset)
}

fn write_loop(shared: &Shared, mut file: File, mut last: u64, receiver: &mpsc::Receiver<Entry>) {
    while let Ok(first) = receiver.recv() {
        let mut batch = vec![first];
        while batch.len() < MAX_WRITE_BATCH {
            match receiver.try_recv() {
                Ok(entry) => batch.push(entry),
                Err(_) => break,
            }
        }
        let mut bytes = Vec::with_capacity(batch.len() * 256);
        let mut offset = last;
        for entry in &batch {
            offset += 1;
            let line = Line {
                runner: &shared.runner,
                offset,
                ts: rfc3339_millis(SystemTime::now()),
                entry,
            };
            if serde_json::to_writer(&mut bytes, &line).is_err() {
                // Every field is a plain string, number, or null; this cannot fail, and if it
                // ever did the offset is simply not consumed.
                offset -= 1;
                continue;
            }
            bytes.push(b'\n');
        }
        // A journal that cannot be written is retried rather than dropped: these lines are
        // what usage and billing are built from, and the disk coming back is the common case.
        loop {
            match append(&mut file, &bytes) {
                Ok(()) => break,
                Err(error) => {
                    eprintln!("soma-api: runner journal write failed, retrying: {error}");
                    thread::sleep(WRITE_RETRY);
                }
            }
        }
        last = offset;
        shared.durable.store(last, Ordering::Release);
        shared.appended.notify_one();
    }
}

/// Appends and syncs, cutting back any partial write so a retry never leaves a torn line.
fn append(file: &mut File, bytes: &[u8]) -> io::Result<()> {
    let before = file.metadata()?.len();
    let result = file.write_all(bytes).and_then(|()| file.sync_data());
    if result.is_err() {
        let _restored = file.set_len(before);
    }
    result
}

pub(crate) fn corrupt(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("journal: {message}"))
}

#[cfg(test)]
#[path = "journal_tests.rs"]
pub(crate) mod tests;
