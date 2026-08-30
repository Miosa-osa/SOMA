//! Bounded capture of one tool's standard output and standard error.
//!
//! Each pipe is drained by its own detached reader that retains at most [`CAPTURE_LIMIT`]
//! bytes, so a chatty tool cannot grow the caller.
//! The first reader to reach that ceiling also asks the supervisor to terminate the group, so
//! a tool that floods its output stops running instead of being read and discarded until its
//! deadline, and the overflow is reported rather than silently shortening the result.
//! The readers are detached rather than scoped: terminating the process group closes their
//! pipes and they finish at once, but the collector still refuses to wait past its own grace,
//! so no caller can block forever on a descriptor held outside the group.

use std::io::Read;
use std::process::{ChildStderr, ChildStdout};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

/// Retained bytes per stream; a stream that reaches this ceiling terminates the group.
pub const CAPTURE_LIMIT: usize = 64 * 1024;
const STDOUT: usize = 0;
const STDERR: usize = 1;

/// One reader's complete report.
struct Report {
    stream: usize,
    retained: Vec<u8>,
    overflowed: bool,
}

/// What the readers retained.
#[derive(Default)]
pub(crate) struct Captured {
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    /// Whether either stream produced more than [`CAPTURE_LIMIT`] bytes.
    pub(crate) overflowed: bool,
}

/// The detached readers of one tool invocation.
pub(crate) struct Readers {
    receiver: Receiver<Report>,
    pending: usize,
    captured: Captured,
}

impl Readers {
    /// Starts one detached reader per present pipe.
    ///
    /// `cancel` is the supervisor's cancellation channel; a reader signals it once, the first
    /// time its stream exceeds [`CAPTURE_LIMIT`].
    pub(crate) fn spawn(
        stdout: Option<ChildStdout>,
        stderr: Option<ChildStderr>,
        cancel: &Sender<()>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let mut pending = 0;
        if let Some(pipe) = stdout {
            pending += 1;
            read_into(pipe, STDOUT, sender.clone(), cancel.clone());
        }
        if let Some(pipe) = stderr {
            pending += 1;
            read_into(pipe, STDERR, sender, cancel.clone());
        }
        Self {
            receiver,
            pending,
            captured: Captured::default(),
        }
    }

    /// Collects whatever the readers have reported by `until` and returns whether all did.
    ///
    /// The call may be repeated after the group is terminated; every reader reports once.
    pub(crate) fn collect(&mut self, until: Instant) -> bool {
        while self.pending > 0 {
            let remaining = until.saturating_duration_since(Instant::now());
            let Ok(report) = self.receiver.recv_timeout(remaining) else {
                return false;
            };
            if report.stream == STDOUT {
                self.captured.stdout = report.retained;
            } else {
                self.captured.stderr = report.retained;
            }
            self.captured.overflowed |= report.overflowed;
            self.pending -= 1;
        }
        true
    }

    /// Returns the retained buffers.
    pub(crate) fn take(self) -> Captured {
        self.captured
    }
}

fn read_into(
    mut source: impl Read + Send + 'static,
    stream: usize,
    sender: Sender<Report>,
    cancel: Sender<()>,
) {
    thread::spawn(move || {
        let mut retained = Vec::new();
        let mut overflowed = false;
        let mut buffer = [0_u8; 8192];
        loop {
            match source.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    let room = CAPTURE_LIMIT.saturating_sub(retained.len());
                    retained.extend_from_slice(&buffer[..count.min(room)]);
                    if count > room && !overflowed {
                        overflowed = true;
                        let _ = cancel.send(());
                    }
                }
            }
        }
        let _ = sender.send(Report {
            stream,
            retained,
            overflowed,
        });
    });
}
