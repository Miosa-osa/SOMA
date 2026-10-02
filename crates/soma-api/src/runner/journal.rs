use std::{
    fs::{File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::runner::clock::rfc3339_millis;

/// How many queued entries one buffered write covers at most.
const MAX_WRITE_BATCH: usize = 4_096;
/// How often the writer syncs what it has written. The sync is the writer's own business and
/// never sits between a request and its response (contract C4).
const SYNC_INTERVAL: Duration = Duration::from_secs(1);
const WRITE_RETRY: Duration = Duration::from_secs(1);
const WRITE_BUFFER_BYTES: usize = 256 * 1024;

/// What one journal line records (contract C4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Create,
    Exec,
    Destroy,
    /// A sandbox the runner's own sweep ended: past its timeout, or its tenant suspended.
    Expire,
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
    boot_epoch: &'a str,
    offset: u64,
    ts: String,
    #[serde(flatten)]
    entry: &'a Entry,
}

#[derive(Deserialize)]
pub(crate) struct OffsetOnly {
    pub(crate) offset: u64,
}

/// The append-only paperwork journal of one runner process.
///
/// Every process start is a new boot epoch with its own file,
/// `<runner>.<boot_epoch>.ndjson`, whose offsets start at 1. The idempotency key is
/// `(runner, boot_epoch, offset)`, so a restart can never collide with lines already acked,
/// and nothing has to be recovered from the previous file before serving. Earlier epochs stay
/// on disk until the shipper has delivered them.
///
/// Entries are queued without blocking and written by one thread into a buffer that is
/// flushed per batch and synced on an interval.
#[derive(Clone)]
pub struct Journal {
    sender: mpsc::Sender<Entry>,
    shared: Arc<Shared>,
}

struct Shared {
    directory: PathBuf,
    path: PathBuf,
    runner: String,
    epoch: String,
    /// The highest offset handed to the operating system.
    written: AtomicU64,
    appended: tokio::sync::Notify,
}

impl Journal {
    /// Starts a new boot epoch file in `directory` and its writer thread.
    ///
    /// # Errors
    ///
    /// Returns the failure to create the directory or the file, or a refusal when the
    /// directory is on a memory-backed filesystem that would lose the journal on reboot.
    pub fn open(directory: &Path, runner: &str) -> io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        refuse_volatile(directory)?;
        let epoch = boot_epoch();
        let path = directory.join(format!("{runner}.{epoch}.ndjson"));
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)?;
        let shared = Arc::new(Shared {
            directory: directory.to_owned(),
            path,
            runner: runner.to_owned(),
            epoch,
            written: AtomicU64::new(0),
            appended: tokio::sync::Notify::new(),
        });
        let (sender, receiver) = mpsc::channel();
        let writer = Arc::clone(&shared);
        thread::Builder::new()
            .name("soma-runner-journal".to_owned())
            .spawn(move || write_loop(&writer, file, &receiver))?;
        Ok(Self { sender, shared })
    }

    /// Queues one entry. It never blocks the caller.
    pub fn record(&self, entry: Entry) {
        // The writer thread only stops when every sender is gone, so a send cannot fail while
        // this handle exists.
        let _queued = self.sender.send(entry);
    }

    /// The highest offset of this epoch that the operating system has.
    #[must_use]
    pub fn written_offset(&self) -> u64 {
        self.shared.written.load(Ordering::Acquire)
    }

    /// Waits until the writer publishes a new written offset.
    pub async fn appended(&self) {
        self.shared.appended.notified().await;
    }

    /// This epoch's file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.shared.path
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.shared.directory
    }

    #[must_use]
    pub fn runner(&self) -> &str {
        &self.shared.runner
    }

    #[must_use]
    pub fn epoch(&self) -> &str {
        &self.shared.epoch
    }
}

/// A new boot epoch: milliseconds since 1970, zero padded so epochs sort by start time, plus
/// random bits so two starts in one millisecond still differ.
fn boot_epoch() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let random = uuid::Uuid::new_v4().simple().to_string();
    format!("{millis:013}-{}", &random[..8])
}

fn write_loop(shared: &Shared, file: File, receiver: &mpsc::Receiver<Entry>) {
    let mut out = BufWriter::with_capacity(WRITE_BUFFER_BYTES, file);
    let mut offset = 0_u64;
    let mut last_sync = Instant::now();
    let mut unsynced = false;
    loop {
        let first = match receiver.recv_timeout(SYNC_INTERVAL) {
            Ok(entry) => Some(entry),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let mut batch: Vec<Entry> = first.into_iter().collect();
        while !batch.is_empty() && batch.len() < MAX_WRITE_BATCH {
            match receiver.try_recv() {
                Ok(entry) => batch.push(entry),
                Err(_) => break,
            }
        }
        if !batch.is_empty() {
            let bytes = encode(shared, &batch, offset);
            // A journal that cannot be written is retried rather than dropped: these lines are
            // what usage and billing are built from, and the disk coming back is the common case.
            while let Err(error) = out.write_all(&bytes).and_then(|()| out.flush()) {
                eprintln!("soma-api: runner journal write failed, retrying: {error}");
                thread::sleep(WRITE_RETRY);
            }
            offset += batch.len() as u64;
            unsynced = true;
            shared.written.store(offset, Ordering::Release);
            shared.appended.notify_one();
        }
        if unsynced && last_sync.elapsed() >= SYNC_INTERVAL {
            if let Err(error) = out.get_ref().sync_data() {
                eprintln!("soma-api: runner journal sync failed: {error}");
            }
            last_sync = Instant::now();
            unsynced = false;
        }
    }
    let _flushed = out.flush().and_then(|()| out.get_ref().sync_data());
}

fn encode(shared: &Shared, batch: &[Entry], after: u64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(batch.len() * 320);
    for (index, entry) in (1_u64..).zip(batch) {
        let line = Line {
            runner: &shared.runner,
            boot_epoch: &shared.epoch,
            offset: after + index,
            ts: rfc3339_millis(SystemTime::now()),
            entry,
        };
        // Every field is a plain string, number, or null, which always encodes.
        let _encoded = serde_json::to_writer(&mut bytes, &line);
        bytes.push(b'\n');
    }
    bytes
}

pub(crate) fn corrupt(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("journal: {message}"))
}

/// Refuses a journal directory on tmpfs or ramfs, where a reboot would lose unshipped usage.
#[cfg(target_os = "linux")]
fn refuse_volatile(directory: &Path) -> io::Result<()> {
    let directory = std::fs::canonicalize(directory)?;
    let mounts = std::fs::read_to_string("/proc/self/mounts")?;
    let filesystem = mounts
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let _device = fields.next()?;
            Some((PathBuf::from(fields.next()?), fields.next()?.to_owned()))
        })
        .filter(|(mount, _)| directory.starts_with(mount))
        .max_by_key(|(mount, _)| mount.components().count())
        .map(|(_, filesystem)| filesystem);
    match filesystem.as_deref() {
        Some("tmpfs" | "ramfs") => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the runner journal must be on persistent disk, not tmpfs",
        )),
        _ => Ok(()),
    }
}

#[cfg(not(target_os = "linux"))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "matches the Linux check, which can refuse"
)]
const fn refuse_volatile(_directory: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
#[path = "journal_tests.rs"]
pub(crate) mod tests;
