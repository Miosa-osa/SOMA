//! T2: the agent outranks the workload it supervises.
//!
//! The agent is PID 1 and the supervisor of every command a sandbox runs, but it starts at the
//! kernel's default scheduling policy, which is the same policy a workload gets. On an N-vCPU
//! guest, N commands that each spin in a loop are N threads of equal priority to the agent, and
//! the agent's own control loop loses the CPU it needs to answer the next request: every exec then
//! stalls or times out, permanently, because the loops never end. Raising the agent above the
//! workload's band is what keeps the control path alive.
//!
//! `SCHED_FIFO` at a fixed priority is the smallest change that does it. It is asked for at agent
//! start and reported either way: a kernel that refuses the request (no `CAP_SYS_NICE`, or a
//! cgroup that forbids the band) leaves the agent exactly where it was, which is slow under the
//! loops but never wrong. Children are dropped back to the default policy so a runaway command
//! cannot hold the realtime band against the agent that supervises it.

#![allow(unsafe_code)]

use std::{fmt, io, os::unix::process::CommandExt as _, process::Command};

/// The realtime priority the agent asks for.
///
/// Above the default band and below anything the machine reserves: high enough that a spinning
/// command cannot starve the agent, low enough that the agent never outranks a host-side decision.
pub const PRIORITY: i32 = 10;

// SCHED_FIFO priorities are 1..=99; the request is inside the band the kernel accepts.
const _: () = assert!(PRIORITY > 0 && PRIORITY < 99);

/// What the priority step achieved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Priority {
    /// The agent runs at `SCHED_FIFO` with this priority.
    Realtime(i32),
    /// The request was refused with this kernel errno; the agent runs at the default policy.
    Default(i32),
}

impl fmt::Display for Priority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Realtime(priority) => write!(formatter, "SCHED_FIFO:{priority}"),
            Self::Default(errno) => write!(formatter, "SCHED_OTHER (refused errno {errno})"),
        }
    }
}

/// Raises the calling thread to `SCHED_FIFO` at [`PRIORITY`], or reports the errno it was refused
/// with.
#[must_use]
pub fn raise() -> Priority {
    let parameters = scheduler_param(PRIORITY);
    let result = set_scheduler(libc::SCHED_FIFO, &parameters);
    if result == 0 {
        Priority::Realtime(PRIORITY)
    } else {
        Priority::Default(last_errno())
    }
}

/// A `sched_param` with `sched_priority` set and every other field zero.
///
/// The struct carries fields beyond the priority on some targets (the deadline parameters), so it
/// is zeroed first and the one field that matters is written; the kernel ignores the rest for the
/// two policies this module uses.
fn scheduler_param(priority: i32) -> libc::sched_param {
    // SAFETY: `sched_param` is a plain C struct of integers and two `timespec`s, every one of
    // which is valid when zero.
    let mut parameters: libc::sched_param = unsafe { std::mem::zeroed() };
    parameters.sched_priority = priority;
    parameters
}

/// Sets the calling thread's scheduling policy through the syscall directly.
///
/// The raw syscall is used rather than the C library wrapper. Measured 2026-10-08, a static musl
/// binary calling `sched_setscheduler` returns `ENOSYS` (38) on a kernel that implements the
/// request, while the same binary calling `syscall(SYS_sched_setscheduler, ...)` with the same
/// three arguments returns the kernel's own answer: musl's `sched_setscheduler` is a stub, and on
/// a machine contract v2 guest the stub made the agent report "refused errno 38" and never take
/// the realtime band. glibc's wrapper reaches the kernel and returns `EPERM` (1) for the same
/// request, so the wrapper is the only difference. Going through the syscall removes it from the
/// path.
fn set_scheduler(policy: libc::c_int, parameters: &libc::sched_param) -> libc::c_long {
    // SAFETY: `syscall` forwards its arguments to the kernel; the request is one `sched_setscheduler`
    // with a pid of zero (this thread), a fixed policy, and a pointer to a live parameter local,
    // and it writes no memory the caller can observe except the scheduler state.
    unsafe {
        libc::syscall(
            libc::SYS_sched_setscheduler,
            0,
            policy,
            std::ptr::from_ref::<libc::sched_param>(parameters),
        )
    }
}

/// Drops the calling thread to the default `SCHED_OTHER` policy.
///
/// The executor calls this from a forked child, so it uses only the syscall and the errno.
///
/// # Errors
///
/// Returns the kernel errno if the policy could not be set.
pub fn drop_to_default() -> io::Result<()> {
    let parameters = scheduler_param(0);
    // As in `raise`, through the syscall directly. It runs in the forked child before `execve`,
    // where only syscalls are safe.
    let result = set_scheduler(libc::SCHED_OTHER, &parameters);
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(last_errno()))
    }
}

/// The current errno as a plain integer.
fn last_errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Arranges for `command`'s child to run at the default policy, not the agent's realtime band.
///
/// A forked child inherits the parent's scheduling policy across `execve`, so a workload would
/// otherwise start at the agent's realtime priority. The refusal is ignored, so a machine that
/// forbids the band still runs the command at whatever the child inherited.
pub fn reset_child_policy(command: &mut Command) {
    // SAFETY: the closure runs in the forked child between `fork` and `execve`, where only
    // async-signal-safe calls are permitted; it is one `sched_setscheduler` syscall whose result is
    // ignored, and it allocates nothing.
    unsafe {
        command.pre_exec(|| {
            let _ = drop_to_default();
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_names_both_outcomes() {
        assert_eq!(Priority::Realtime(10).to_string(), "SCHED_FIFO:10");
        assert_eq!(
            Priority::Default(libc::EPERM).to_string(),
            format!("SCHED_OTHER (refused errno {})", libc::EPERM)
        );
    }

    #[test]
    fn the_priority_is_inside_the_realtime_band() {
        // A compile-time assertion above pins the band; this names the value in the test output.
        assert_eq!(PRIORITY, 10);
    }
}
