//! Bounded capture of one tool's standard output and standard error.
//!
//! Each pipe is drained by its own detached reader that retains at most [`CAPTURE_LIMIT`] bytes
//! and discards the rest, so a chatty tool cannot grow the compiler.
//! The readers are detached rather than scoped: terminating the process group closes their
//! pipes and they finish at once, but the collector still refuses to wait past its own grace,
//! so no build can block forever on a descriptor held outside the group.

use std::io::Read;
use std::process::{ChildStderr, ChildStdout};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

/// Retained bytes per stream; everything beyond is read and discarded.
pub(super) const CAPTURE_LIMIT: usize = 64 * 1024;
const STDOUT: usize = 0;
const STDERR: usize = 1;

/// What the readers retained.
#[derive(Default)]
pub(super) struct Captured {
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
    /// Whether every reader reported before the collection grace elapsed.
    pub(super) complete: bool,
}

/// The detached readers of one tool invocation.
pub(super) struct Readers {
    receiver: Receiver<(usize, Vec<u8>)>,
    pending: usize,
    captured: Captured,
}

impl Readers {
    /// Starts one detached reader per present pipe.
    pub(super) fn spawn(stdout: Option<ChildStdout>, stderr: Option<ChildStderr>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let mut pending = 0;
        if let Some(pipe) = stdout {
            pending += 1;
            read_into(pipe, STDOUT, sender.clone());
        }
        if let Some(pipe) = stderr {
            pending += 1;
            read_into(pipe, STDERR, sender);
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
    pub(super) fn collect(&mut self, until: Instant) -> bool {
        while self.pending > 0 {
            let remaining = until.saturating_duration_since(Instant::now());
            match self.receiver.recv_timeout(remaining) {
                Ok((STDOUT, bytes)) => self.captured.stdout = bytes,
                Ok((_, bytes)) => self.captured.stderr = bytes,
                Err(_) => return false,
            }
            self.pending -= 1;
        }
        self.captured.complete = true;
        true
    }

    /// Returns the retained buffers.
    pub(super) fn take(self) -> Captured {
        self.captured
    }
}

fn read_into(
    mut source: impl Read + Send + 'static,
    stream: usize,
    sender: Sender<(usize, Vec<u8>)>,
) {
    thread::spawn(move || {
        let mut retained = Vec::new();
        let mut buffer = [0_u8; 8192];
        loop {
            match source.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    let room = CAPTURE_LIMIT.saturating_sub(retained.len());
                    retained.extend_from_slice(&buffer[..count.min(room)]);
                }
            }
        }
        let _ = sender.send((stream, retained));
    });
}
