//! The single bounded poll loop over one child's standard output and standard error.
//!
//! There is no queue and no reader thread: both pipes are polled together and every read goes
//! into one reused fixed buffer whose length is [`RESIDENT_OUTPUT_BYTES`].
//! [`OutputBudget::room`] bounds each read to the unspent allowance plus one probe byte, so the
//! agent's resident memory for a command is the fixed buffer plus at most one admitted chunk,
//! whatever the child produces.
//! The moment the allowance is reached or the sink fails, the complete process group is killed
//! and the loop switches to a drain that is bounded by [`KILL_GRACE`].

#![allow(unsafe_code)]

use std::fs::File;
use std::io::{ErrorKind, Read as _};
use std::os::fd::{AsRawFd as _, OwnedFd, RawFd};
use std::process::{ChildStderr, ChildStdout};
use std::time::Instant;

use crate::descendants;
use crate::output::OutputBudget;

use super::{KILL_GRACE, OutputSink, RESIDENT_OUTPUT_BYTES, SinkFault};

/// Upper bound on one `poll` wait so a lost wake-up cannot stall the loop.
const POLL_CEILING_MILLIS: libc::c_int = 250;

/// The one stream identity carried alongside each source.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Stream {
    Stdout,
    Stderr,
}

/// What the bounded loop observed.
pub(super) struct Streamed {
    pub(super) limit_hit: bool,
    pub(super) timed_out: bool,
    pub(super) sink_failed: bool,
    pub(super) stdout_bytes: u64,
    pub(super) stderr_bytes: u64,
    pub(super) kill_by: Instant,
}

struct Source {
    file: File,
    stream: Stream,
}

/// Streams admitted output into `sink` until both pipes close or a bound is reached.
pub(super) fn stream(
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    budget: &mut OutputBudget,
    sink: &mut impl OutputSink,
    deadline: Instant,
    process_group: i32,
) -> Streamed {
    let mut sources: Vec<Source> = [
        stdout.map(|pipe| Source::new(OwnedFd::from(pipe), Stream::Stdout)),
        stderr.map(|pipe| Source::new(OwnedFd::from(pipe), Stream::Stderr)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut observed = Streamed {
        limit_hit: false,
        timed_out: false,
        sink_failed: false,
        stdout_bytes: 0,
        stderr_bytes: 0,
        kill_by: deadline,
    };
    let mut buffer = [0_u8; RESIDENT_OUTPUT_BYTES];
    let mut killed = false;
    while !sources.is_empty() {
        let Some(timeout) = wait_millis(observed.kill_by) else {
            if killed {
                break;
            }
            observed.timed_out = true;
            killed = true;
            kill(&mut observed, process_group);
            continue;
        };
        let mut fds: Vec<libc::pollfd> = sources.iter().map(Source::pollfd).collect();
        if !poll(&mut fds, timeout) {
            continue;
        }
        // Every ready stream reads a share of the remaining room in this pass, so a fast
        // writer on one pipe cannot spend the whole allowance before the other is read.
        let ready = fds.iter().filter(|fd| fd.revents != 0).count().max(1);
        let mut index = 0;
        while index < sources.len() {
            if fds[index].revents == 0 {
                index += 1;
                continue;
            }
            let outcome = if killed {
                Ok(sources[index].discard(&mut buffer))
            } else {
                sources[index].admit(&mut buffer, ready, budget, sink, &mut observed)
            };
            observed.sink_failed |= outcome.is_err();
            let open = outcome.unwrap_or(false);
            if (observed.sink_failed || observed.limit_hit) && !killed {
                killed = true;
                kill(&mut observed, process_group);
            }
            if open {
                index += 1;
            } else {
                sources.remove(index);
                fds.remove(index);
            }
        }
    }
    observed
}

fn kill(observed: &mut Streamed, process_group: i32) {
    descendants::kill_group(process_group);
    observed.kill_by = Instant::now() + KILL_GRACE;
}

/// Returns the bounded `poll` timeout, or `None` when the current bound has elapsed.
fn wait_millis(until: Instant) -> Option<libc::c_int> {
    let remaining = until.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return None;
    }
    let millis = libc::c_int::try_from(remaining.as_millis().max(1)).unwrap_or(POLL_CEILING_MILLIS);
    Some(millis.min(POLL_CEILING_MILLIS))
}

/// Waits for readiness and returns whether the caller should inspect `fds`.
fn poll(fds: &mut [libc::pollfd], timeout: libc::c_int) -> bool {
    let count = libc::nfds_t::try_from(fds.len()).unwrap_or(0);
    // SAFETY: the pointer and count describe exactly the caller's live slice of `pollfd`
    // values, each holding a descriptor this loop owns for the duration of the call.
    let ready = unsafe { libc::poll(fds.as_mut_ptr(), count, timeout) };
    ready > 0
}

impl Source {
    fn new(fd: OwnedFd, stream: Stream) -> Self {
        set_nonblocking(fd.as_raw_fd());
        Self {
            file: File::from(fd),
            stream,
        }
    }

    fn pollfd(&self) -> libc::pollfd {
        libc::pollfd {
            fd: self.file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }
    }

    /// Reads at most this stream's share of the reserved room and delivers the admitted prefix.
    ///
    /// `ready` is the number of streams the current pass found readable, so the share always
    /// leaves room for the other readable stream in the same pass.
    fn admit(
        &mut self,
        buffer: &mut [u8; RESIDENT_OUTPUT_BYTES],
        ready: usize,
        budget: &mut OutputBudget,
        sink: &mut impl OutputSink,
        observed: &mut Streamed,
    ) -> Result<bool, SinkFault> {
        let room = budget.room().div_ceil(ready).max(1).min(budget.room());
        let Some(read) = self.read(&mut buffer[..room]) else {
            return Ok(false);
        };
        if read == 0 {
            return Ok(true);
        }
        let reservation = budget.reserve(read);
        if reservation.admitted > 0 {
            let chunk = &buffer[..reservation.admitted];
            let delivered = u64::try_from(chunk.len()).unwrap_or(0);
            match self.stream {
                Stream::Stdout => {
                    sink.stdout(chunk)?;
                    observed.stdout_bytes = observed.stdout_bytes.saturating_add(delivered);
                }
                Stream::Stderr => {
                    sink.stderr(chunk)?;
                    observed.stderr_bytes = observed.stderr_bytes.saturating_add(delivered);
                }
            }
        }
        observed.limit_hit |= reservation.reached_limit;
        Ok(true)
    }

    /// Reads and drops bytes after the group was killed so a blocked writer can die.
    fn discard(&mut self, buffer: &mut [u8; RESIDENT_OUTPUT_BYTES]) -> bool {
        self.read(buffer).is_some()
    }

    /// Returns the byte count, or `None` when the pipe reached its end or failed.
    fn read(&mut self, destination: &mut [u8]) -> Option<usize> {
        match self.file.read(destination) {
            Ok(0) => None,
            Ok(count) => Some(count),
            Err(error)
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
            {
                Some(0)
            }
            Err(_) => None,
        }
    }
}

fn set_nonblocking(fd: RawFd) {
    // SAFETY: `fcntl` with `F_SETFL` takes one integer flag word and only changes the file
    // status flags of the descriptor this loop owns.
    unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) };
}
