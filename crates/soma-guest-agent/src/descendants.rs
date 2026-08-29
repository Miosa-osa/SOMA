//! Process-group termination and complete descendant reaping.
//!
//! A command runs in its own process group so one signal reaches every member.
//! As PID 1 the agent additionally sweeps every other process in the guest, which by
//! construction is a descendant, so nothing survives a command or a shutdown.

#![allow(unsafe_code)]

use std::fs;
use std::io;

use crate::pid1;

const INIT_PID: i32 = 1;

/// Sends `SIGKILL` to every member of the process group.
///
/// Group identifiers at or below the init process are never signalled.
pub fn kill_group(pgid: i32) {
    if pgid <= INIT_PID {
        return;
    }
    // SAFETY: `kill` with a negative process-group identifier above one has no memory
    // preconditions and can only reach processes in that group.
    unsafe { libc::kill(-pgid, libc::SIGKILL) };
}

/// Blocks until every child in the process group has been reaped and returns the count.
///
/// Callers must kill the group first so this loop is bounded by kernel exit latency.
pub fn reap_group(pgid: i32) -> usize {
    if pgid <= INIT_PID {
        return 0;
    }
    let mut reaped = 0;
    loop {
        let mut status = 0;
        // SAFETY: `waitpid` writes one integer into the valid local; a negative identifier
        // above one selects only children in that process group.
        let child = unsafe { libc::waitpid(-pgid, &raw mut status, 0) };
        if child > 0 {
            reaped += 1;
            continue;
        }
        if child < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return reaped;
    }
}

/// As PID 1, kills every other process in the guest and reaps all of them.
///
/// Outside PID 1 this is a no-op so host tests never signal unrelated processes.
pub fn sweep_strays() -> usize {
    if !pid1::is_pid1() {
        return 0;
    }
    for pid in other_pids() {
        // SAFETY: `kill` with a positive identifier has no memory preconditions; the identifier
        // was read from `/proc` and is neither this process nor a reserved value.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    let mut reaped = 0;
    loop {
        let mut status = 0;
        // SAFETY: `waitpid` writes one integer into the valid local; `-1` reaps any child,
        // which is every remaining process because this is PID 1.
        let pid = unsafe { libc::waitpid(-1, &raw mut status, 0) };
        if pid > 0 {
            reaped += 1;
            continue;
        }
        if pid < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return reaped;
    }
}

/// Lists every process identifier in `/proc` other than this process and init.
#[must_use]
pub fn other_pids() -> Vec<i32> {
    let own = i32::try_from(std::process::id()).unwrap_or(INIT_PID);
    fs::read_dir("/proc")
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
                .filter(|pid| *pid > INIT_PID && *pid != own)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_group_identifiers_are_never_signalled_or_awaited() {
        kill_group(0);
        kill_group(1);
        kill_group(-5);
        assert_eq!(reap_group(0), 0);
        assert_eq!(reap_group(1), 0);
    }

    #[test]
    fn stray_sweeps_are_inert_outside_pid_one() {
        assert_eq!(sweep_strays(), 0);
    }

    #[test]
    fn other_pids_excludes_this_process_and_init() {
        let own = i32::try_from(std::process::id()).expect("pid fits");
        let pids = other_pids();
        assert!(!pids.contains(&own));
        assert!(!pids.contains(&1));
    }
}
