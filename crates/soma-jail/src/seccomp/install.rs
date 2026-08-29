//! Linux `x86_64` installation of an assembled filter on the calling process.

#![allow(unsafe_code)]

use std::{error::Error, fmt, io};

use super::{FilterProgram, Phase, program_for};

/// Typed installation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeccompError {
    NoNewPrivs(i32),
    Install(i32),
    /// `SECCOMP_FILTER_FLAG_TSYNC` could not synchronize this thread.
    ThreadSyncFailed(i64),
}

impl fmt::Display for SeccompError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoNewPrivs(errno) => {
                write!(formatter, "PR_SET_NO_NEW_PRIVS failed: errno {errno}")
            }
            Self::Install(errno) => {
                write!(formatter, "SECCOMP_SET_MODE_FILTER failed: errno {errno}")
            }
            Self::ThreadSyncFailed(tid) => {
                write!(
                    formatter,
                    "seccomp filter could not synchronize thread {tid}"
                )
            }
        }
    }
}

impl Error for SeccompError {}

fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Sets `no_new_privs` and installs the `phase` filter on every thread of this process.
///
/// # Errors
///
/// Returns a [`SeccompError`] if either step fails; nothing is installed partially because the
/// kernel applies a filter atomically.
pub fn install_filter(phase: Phase) -> Result<(), SeccompError> {
    install_sock_filters(&to_sock_filters(&program_for(phase)))
}

/// Converts an assembled program into the kernel's instruction layout.
pub(crate) fn to_sock_filters(program: &FilterProgram) -> Vec<libc::sock_filter> {
    program
        .instructions()
        .iter()
        .map(|instruction| libc::sock_filter {
            code: instruction.code,
            jt: instruction.jt,
            jf: instruction.jf,
            k: instruction.k,
        })
        .collect()
}

/// Installs an already converted program; allocation-free so the cloned child can call it.
pub(crate) fn install_sock_filters(filter: &[libc::sock_filter]) -> Result<(), SeccompError> {
    // SAFETY: `PR_SET_NO_NEW_PRIVS` takes only integer arguments and touches no memory.
    let result = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if result != 0 {
        return Err(SeccompError::NoNewPrivs(errno()));
    }
    let len = u16::try_from(filter.len()).map_err(|_| SeccompError::Install(libc::EINVAL))?;
    let fprog = libc::sock_fprog {
        len,
        filter: filter.as_ptr().cast_mut(),
    };
    // SAFETY: `fprog` points at `filter`, which stays alive and unmoved for the whole call, and
    // `len` is its exact instruction count; the kernel copies the program before returning.
    let result = unsafe {
        libc::syscall(
            libc::SYS_seccomp,
            libc::SECCOMP_SET_MODE_FILTER,
            libc::SECCOMP_FILTER_FLAG_TSYNC,
            std::ptr::from_ref(&fprog),
        )
    };
    match result {
        0 => Ok(()),
        -1 => Err(SeccompError::Install(errno())),
        tid => Err(SeccompError::ThreadSyncFailed(tid)),
    }
}
