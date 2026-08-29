//! `clone3` into the namespaces and cgroup, and the parent's read of the failure report.

#![allow(unsafe_code)]

use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    time::Instant,
};

use super::{
    child::{self, ChildPlan},
    failure::{ChildFailure, REPORT_BYTES},
    launch_error::LaunchError,
};
use crate::{cgroup::CgroupLeaf, namespaces::CLONE_NAMESPACES};

const CLONE_PIDFD: u64 = 0x1000;
const CLONE_INTO_CGROUP: u64 = 0x2_0000_0000;

fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Clones the child directly into its six namespaces and the cgroup leaf with a pidfd.
///
/// In the child this never returns; in the parent it returns the PID and the owned pidfd.
pub(super) fn clone_child(
    plan: &ChildPlan,
    cgroup: &CgroupLeaf,
) -> Result<(i32, OwnedFd), LaunchError> {
    let mut pidfd: libc::c_int = -1;
    // SAFETY: `clone_args` is plain data; zero is a valid initial value for every field.
    let mut args: libc::clone_args = unsafe { std::mem::zeroed() };
    args.flags = CLONE_NAMESPACES | CLONE_PIDFD | CLONE_INTO_CGROUP;
    args.pidfd = (&raw mut pidfd) as u64;
    args.exit_signal = u64::try_from(libc::SIGCHLD).unwrap_or(17);
    args.cgroup =
        u64::try_from(cgroup.dir_fd().as_raw_fd()).map_err(|_| LaunchError::Clone(libc::EBADF))?;
    // SAFETY: `args` is a fully initialized `clone_args` of the exact size passed, `pidfd`
    // outlives the call, and the child immediately runs `child::run`, which never returns and
    // never allocates, so the multithreaded parent's locks cannot deadlock it.
    let result = unsafe {
        libc::syscall(
            libc::SYS_clone3,
            &raw const args,
            size_of::<libc::clone_args>(),
        )
    };
    match result {
        0 => child::run(plan),
        pid if pid > 0 => {
            // SAFETY: `CLONE_PIDFD` stored a fresh descriptor that nothing else owns.
            let pidfd = unsafe { OwnedFd::from_raw_fd(pidfd) };
            Ok((i32::try_from(pid).unwrap_or(i32::MAX), pidfd))
        }
        _ => Err(LaunchError::Clone(errno())),
    }
}

/// Reads the child's failure report; EOF without bytes means `execveat` succeeded.
pub(super) fn read_report(
    report: &OwnedFd,
    deadline: Instant,
) -> Result<Option<ChildFailure>, LaunchError> {
    let mut bytes = [0u8; REPORT_BYTES];
    let mut filled = 0usize;
    while filled < REPORT_BYTES {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(LaunchError::Report(libc::ETIMEDOUT));
        }
        let mut poll = libc::pollfd {
            fd: report.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout =
            libc::c_int::try_from(remaining.as_millis().max(1)).unwrap_or(libc::c_int::MAX);
        // SAFETY: `poll` receives one valid `pollfd` and its count.
        let ready = unsafe { libc::poll(&raw mut poll, 1, timeout) };
        if ready < 0 && errno() != libc::EINTR {
            return Err(LaunchError::Report(errno()));
        }
        if ready <= 0 {
            continue;
        }
        // SAFETY: the destination is the unfilled tail of `bytes` with its exact length.
        let read = unsafe {
            libc::read(
                report.as_raw_fd(),
                bytes[filled..].as_mut_ptr().cast(),
                REPORT_BYTES - filled,
            )
        };
        match read {
            0 => break,
            count if count > 0 => filled += usize::try_from(count).unwrap_or(0),
            _ if errno() == libc::EINTR => {}
            _ => return Err(LaunchError::Report(errno())),
        }
    }
    match filled {
        0 => Ok(None),
        REPORT_BYTES => ChildFailure::decode(bytes)
            .map(Some)
            .ok_or(LaunchError::Report(libc::EBADMSG)),
        _ => Err(LaunchError::Report(libc::EIO)),
    }
}
