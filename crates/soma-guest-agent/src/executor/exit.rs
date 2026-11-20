//! Waiting for one child's exit on a process descriptor, bounded by the command's deadline.
//!
//! A pidfd becomes readable when its process exits, so a `poll` on it returns at the exit itself
//! rather than at the next step of a sleep schedule. The schedule it replaces checked again
//! after 50 microseconds and doubled up to 5 milliseconds, so a command that exited at 4 ms was
//! noticed at about 6.35 ms, and later still when the sleeps overshot on a busy machine (issue
//! #128).

#![allow(unsafe_code)]

use std::io;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::process::Child;
use std::time::Instant;

/// How one watched wait ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Watched {
    /// The child has exited and can be reaped without blocking.
    Exited,
    /// The deadline passed with the child still running.
    Deadline,
    /// The kernel would not watch the child, so the caller must wait some other way.
    Unwatchable,
}

/// Waits until `child` exits or `until` passes.
///
/// The child has not been reaped, so its process identifier cannot have been reused and the
/// descriptor names exactly this child.
pub(super) fn watch(child: &Child, until: Instant) -> Watched {
    let Some(pidfd) = open(child) else {
        return Watched::Unwatchable;
    };
    loop {
        let Some(timeout) = millis_until(until) else {
            return Watched::Deadline;
        };
        let mut entry = libc::pollfd {
            fd: pidfd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `poll` reads and writes exactly the one `pollfd` the pointer names, which is a
        // live local, and the count passed is one.
        let ready = unsafe { libc::poll(&raw mut entry, 1, timeout) };
        if ready > 0 {
            return Watched::Exited;
        }
        if ready < 0 && io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            return Watched::Unwatchable;
        }
    }
}

/// Opens a process descriptor for the child, or `None` when the kernel refuses one.
fn open(child: &Child) -> Option<OwnedFd> {
    let pid = libc::pid_t::try_from(child.id()).ok()?;
    // SAFETY: `pidfd_open` takes a process identifier and a flags word by value and returns a new
    // descriptor or -1; it reads and writes no memory of this process.
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    let fd = libc::c_int::try_from(raw).ok().filter(|fd| *fd >= 0)?;
    // SAFETY: the kernel has just returned this descriptor, close-on-exec, and nothing else owns
    // it.
    Some(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Milliseconds until `until`, rounded up so the wait never ends before it; `None` once it
/// has passed.
fn millis_until(until: Instant) -> Option<libc::c_int> {
    let left = until.checked_duration_since(Instant::now())?;
    if left.is_zero() {
        return None;
    }
    let millis = left.as_micros().div_ceil(1_000);
    Some(libc::c_int::try_from(millis).unwrap_or(libc::c_int::MAX))
}
