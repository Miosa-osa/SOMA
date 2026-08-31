//! One live terminal session: the master end, the shell under it, and the bytes between them.
//!
//! Nothing here queues. A read waits for the first byte and then takes at most one bounded chunk
//! into one buffer sized for exactly that, so a program that prints without end costs the agent
//! the same fixed memory as one that prints nothing. The caller's own reads are what pace it.
//!
//! The master is nonblocking, so the wait is a `poll` the caller sized rather than a read that
//! could sit past every deadline the session has.

#![allow(unsafe_code)]

use std::fs::File;
use std::io::{ErrorKind, Read as _, Write as _};
use std::os::fd::{AsRawFd as _, RawFd};
use std::process::Child;

use soma_guest::{MAX_PTY_CHUNK_BYTES, PtyFailure, PtyOutcome, PtySize};

use crate::descendants;

use super::device;

/// One open pseudo-terminal and the shell running on it.
pub(super) struct Session {
    master: File,
    child: Child,
}

impl Session {
    /// Allocates the pair, spawns the shell as its session leader, and keeps the master.
    ///
    /// The slave is dropped as soon as the child holds it. While the agent still had a copy the
    /// terminal could never reach its end, because the kernel reports that only once no process
    /// has the slave open, and a caller draining output would wait for a flag that never came.
    pub(super) fn open(size: PtySize) -> Result<Self, ()> {
        let pair = device::open(size).map_err(|_| ())?;
        let child = device::spawn(&pair.slave).map_err(|_| ())?;
        drop(pair.slave);
        set_nonblocking(pair.master.as_raw_fd());
        Ok(Self {
            master: pair.master,
            child,
        })
    }

    /// Writes what was typed, and reports how much of it the terminal took.
    ///
    /// A short write is an answer rather than a failure: the terminal's input buffer is finite,
    /// and a caller that pasted more than fits sends the rest on its next request.
    pub(super) fn write(&mut self, bytes: &[u8]) -> PtyOutcome {
        match self.master.write(bytes) {
            Ok(written) => PtyOutcome::Wrote {
                bytes: u32::try_from(written).unwrap_or(u32::MAX),
            },
            Err(error) if error.kind() == ErrorKind::WouldBlock => PtyOutcome::Wrote { bytes: 0 },
            Err(_) => PtyOutcome::Failed(PtyFailure::Failed),
        }
    }

    /// Waits up to `wait_millis` for the first byte and answers with one bounded chunk.
    ///
    /// An empty chunk with the end flag clear means nothing arrived within the wait, which is
    /// the ordinary state of a terminal sitting at a prompt. The end flag is set only once the
    /// shell has exited and its remaining output has been handed over, because the kernel keeps
    /// answering reads from the terminal's buffer after the last writer is gone and only then
    /// reports the hang-up.
    pub(super) fn read(&mut self, wait_millis: u32) -> PtyOutcome {
        if !ready(self.master.as_raw_fd(), wait_millis) {
            return PtyOutcome::Output {
                bytes: Box::default(),
                end: false,
            };
        }
        let mut buffer = vec![0_u8; MAX_PTY_CHUNK_BYTES];
        match self.master.read(&mut buffer) {
            Ok(0) => self.ended(),
            Ok(count) => {
                buffer.truncate(count);
                PtyOutcome::Output {
                    bytes: buffer.into_boxed_slice(),
                    end: false,
                }
            }
            Err(error)
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
            {
                PtyOutcome::Output {
                    bytes: Box::default(),
                    end: false,
                }
            }
            // Every other error on a master end means the last process holding the slave is
            // gone, which the kernel reports as `EIO` rather than as an end of file.
            Err(_) => self.ended(),
        }
    }

    /// Tells the terminal its new dimensions, which delivers `SIGWINCH` to the session.
    pub(super) fn resize(&mut self, size: PtySize) -> PtyOutcome {
        match device::set_size(&self.master, size) {
            Ok(()) => PtyOutcome::Resized(size),
            Err(_) => PtyOutcome::Failed(PtyFailure::Failed),
        }
    }

    /// Ends the session and everything running under it.
    ///
    /// The shell is a session leader, so its process group holds every job started in the
    /// terminal, and killing the group is what stops the ones the shell itself would not.
    pub(super) fn end(mut self) {
        let group = i32::try_from(self.child.id()).unwrap_or(0);
        if group > 1 {
            descendants::kill_group(group);
            descendants::reap_group(group);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Reaps the shell and reports the one end of this stream.
    fn ended(&mut self) -> PtyOutcome {
        let _ = self.child.kill();
        let _ = self.child.wait();
        PtyOutcome::Output {
            bytes: Box::default(),
            end: true,
        }
    }
}

/// Waits for the terminal to have something to say, for at most the caller's bounded wait.
fn ready(fd: RawFd, wait_millis: u32) -> bool {
    let mut poller = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let timeout = libc::c_int::try_from(wait_millis).unwrap_or(libc::c_int::MAX);
    // SAFETY: the pointer and count describe exactly one live `pollfd` holding a descriptor
    // this session owns for the duration of the call.
    let ready = unsafe { libc::poll(&raw mut poller, 1, timeout) };
    // A hang-up is readable news: the buffered output still has to be drained before the end
    // of the stream can be reported, and only a read can tell those two apart.
    ready > 0
}

fn set_nonblocking(fd: RawFd) {
    // SAFETY: `fcntl` with `F_SETFL` takes one integer flag word and only changes the file
    // status flags of the descriptor this session owns.
    unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) };
}
