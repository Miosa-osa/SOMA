//! pidfd-based waiting and signalling; a numeric PID is never used to address the child.

#![allow(unsafe_code)]

use std::{
    error::Error,
    fmt, io,
    os::fd::{AsRawFd, BorrowedFd},
    time::Instant,
};

use crate::evidence::ExitReason;

/// Typed wait failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitError {
    Timeout,
    AlreadyReaped,
    Errno(i32),
}

impl fmt::Display for WaitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => write!(formatter, "the child did not exit before the deadline"),
            Self::AlreadyReaped => write!(formatter, "the child was already reaped"),
            Self::Errno(errno) => write!(formatter, "waiting on the pidfd failed: errno {errno}"),
        }
    }
}

impl Error for WaitError {}

/// Typed signal failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalError {
    /// The process behind the pidfd no longer exists; a reused PID is never targeted.
    Gone,
    Errno(i32),
}

impl fmt::Display for SignalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gone => write!(formatter, "the jailed process is gone"),
            Self::Errno(errno) => write!(formatter, "pidfd_send_signal failed: errno {errno}"),
        }
    }
}

impl Error for SignalError {}

fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Sends `signal` through the pidfd.
pub(crate) fn send_signal(pidfd: BorrowedFd<'_>, signal: i32) -> Result<(), SignalError> {
    // SAFETY: the syscall takes a descriptor, a signal number, a null siginfo, and zero flags.
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            signal,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        match errno() {
            libc::ESRCH => Err(SignalError::Gone),
            other => Err(SignalError::Errno(other)),
        }
    }
}

/// Waits until the pidfd becomes readable or `deadline` passes, then reaps the child.
pub(crate) fn wait_exit(pidfd: BorrowedFd<'_>, deadline: Instant) -> Result<ExitReason, WaitError> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(WaitError::Timeout);
        }
        let timeout =
            libc::c_int::try_from(remaining.as_millis().max(1)).unwrap_or(libc::c_int::MAX);
        let mut poll = libc::pollfd {
            fd: pidfd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `poll` receives one valid `pollfd` and its count.
        let ready = unsafe { libc::poll(&raw mut poll, 1, timeout) };
        match ready {
            1 => break,
            0 => {}
            _ if errno() == libc::EINTR => {}
            _ => return Err(WaitError::Errno(errno())),
        }
    }
    // SAFETY: `siginfo` is zeroed storage the kernel fills on success.
    let mut siginfo: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let id = libc::id_t::try_from(pidfd.as_raw_fd()).map_err(|_| WaitError::Errno(libc::EBADF))?;
    // SAFETY: `P_PIDFD` names the descriptor and `siginfo` is valid writable storage.
    let result = unsafe { libc::waitid(libc::P_PIDFD, id, &raw mut siginfo, libc::WEXITED) };
    if result != 0 {
        return Err(match errno() {
            libc::ECHILD => WaitError::AlreadyReaped,
            other => WaitError::Errno(other),
        });
    }
    // SAFETY: after a successful `WEXITED` wait the child fields of `siginfo` are populated.
    let status = unsafe { siginfo.si_status() };
    Ok(match siginfo.si_code {
        libc::CLD_EXITED => ExitReason::Exited(status),
        libc::CLD_DUMPED => ExitReason::Signaled {
            signal: status,
            core_dumped: true,
        },
        _ => ExitReason::Signaled {
            signal: status,
            core_dumped: false,
        },
    })
}
