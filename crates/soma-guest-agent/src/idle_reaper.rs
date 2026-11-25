//! Reaping orphans while the agent waits for its next request.
//!
//! As PID 1 the agent inherits every process whose parent exits, and until it reaps one the
//! process stays a zombie that `kill -0` still answers for. The executor reaps at the end of
//! each command, which leaves a zombie in place for as long as no command arrives: a process a
//! survivor started (an agent run under a desktop's envd) that ends between commands looks alive
//! to anything watching its pid.
//!
//! This thread reaps those zombies once a second, and only while the serve loop is blocked
//! waiting for a request, so it never takes the exit status of a command the executor is about
//! to wait for. An open terminal's shell is reaped by its own session, which still signals it by
//! pid afterwards, so a terminal's process group is never reaped here: a recycled pid must not
//! be signalled in its place.

#![allow(unsafe_code)]

use std::fs;
use std::sync::{Mutex, MutexGuard, Once, PoisonError};
use std::thread;
use std::time::Duration;

use crate::pid1;

const INTERVAL: Duration = Duration::from_secs(1);

struct State {
    idle: bool,
    kept_groups: Vec<i32>,
}

static STATE: Mutex<State> = Mutex::new(State {
    idle: false,
    kept_groups: Vec::new(),
});
static START: Once = Once::new();

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Starts the reaper thread once, as PID 1 only.
pub fn start() {
    if !pid1::is_pid1() {
        return;
    }
    START.call_once(|| {
        let _ = thread::Builder::new().name("idle-reaper".into()).spawn(|| {
            loop {
                thread::sleep(INTERVAL);
                let state = state();
                if state.idle {
                    reap_exited_orphans(&state.kept_groups);
                }
            }
        });
    });
}

/// Runs `wait`, the serve loop's blocking wait for a request, with reaping allowed.
///
/// Reaping stops before this returns: taking the lock waits out a pass already under way, so the
/// request that ended the wait runs with no reaper active.
pub fn while_idle<T>(wait: impl FnOnce() -> T) -> T {
    state().idle = true;
    let result = wait();
    state().idle = false;
    result
}

/// Keeps a terminal's process group out of reach until its session has reaped its shell.
pub fn keep_group(group: i32) {
    let mut state = state();
    if !state.kept_groups.contains(&group) {
        state.kept_groups.push(group);
    }
}

/// Releases a group kept by [`keep_group`], once its shell has been reaped.
pub fn release_group(group: i32) {
    state().kept_groups.retain(|kept| *kept != group);
}

fn reap_exited_orphans(kept_groups: &[i32]) -> usize {
    let Ok(entries) = fs::read_dir("/proc") else {
        return 0;
    };
    let mut reaped = 0;
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        if !should_reap(&stat, kept_groups) {
            continue;
        }
        let mut status = 0;
        // SAFETY: `waitpid` writes one integer into the valid local and returns at once with
        // `WNOHANG`; a pid that is not this process's child answers `ECHILD` and is left alone.
        if unsafe { libc::waitpid(pid, &raw mut status, libc::WNOHANG) } == pid {
            reaped += 1;
        }
    }
    reaped
}

/// Whether a `/proc/<pid>/stat` line names an exited orphan of init outside every kept group.
fn should_reap(stat: &str, kept_groups: &[i32]) -> bool {
    let Some((state, parent, group)) = parse_stat(stat) else {
        return false;
    };
    state == 'Z' && parent == 1 && !kept_groups.contains(&group)
}

/// The state, parent and process group of a `/proc/<pid>/stat` line.
///
/// The command name sits in parentheses and may itself hold spaces and parentheses, so the
/// fields are read after the last closing one.
fn parse_stat(stat: &str) -> Option<(char, i32, i32)> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut fields = rest.split_ascii_whitespace();
    let state = fields.next()?.chars().next()?;
    let parent = fields.next()?.parse().ok()?;
    let group = fields.next()?.parse().ok()?;
    Some((state, parent, group))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_state_parent_and_group_after_the_command_name() {
        assert_eq!(
            parse_stat("4242 (sh) Z 1 4242 4242 0 -1"),
            Some(('Z', 1, 4242))
        );
        assert_eq!(
            parse_stat("77 (a (b) c) S 12 70 70 0 -1"),
            Some(('S', 12, 70)),
            "a command name with spaces and parentheses"
        );
        assert_eq!(parse_stat("garbage"), None);
    }

    #[test]
    fn reaps_only_exited_orphans_of_init_outside_kept_groups() {
        let kept = [500];
        assert!(
            should_reap("10 (sh) Z 1 10 10 0", &kept),
            "an exited orphan"
        );
        assert!(!should_reap("11 (sh) S 1 11 11 0", &kept), "a live process");
        assert!(
            !should_reap("12 (sh) Z 9 12 12 0", &kept),
            "another process's child"
        );
        assert!(
            !should_reap("501 (bash) Z 1 500 500 0", &kept),
            "a terminal's group"
        );
    }

    #[test]
    fn idle_is_cleared_when_the_wait_returns() {
        assert_eq!(while_idle(|| 7), 7);
        assert!(!state().idle);
    }

    #[test]
    fn the_reaper_never_starts_outside_pid_one() {
        start();
        assert!(!START.is_completed());
    }

    // The thread under test is what reaps this child, as init does for any orphan.
    #[allow(clippy::zombie_processes)]
    #[test]
    fn as_pid_one_an_exited_orphan_is_reaped_while_idle() {
        // Only meaningful as init, which a test run is not; the container run in the
        // browser-default runbook starts this binary as PID 1 to exercise it.
        if !pid1::is_pid1() {
            return;
        }
        start();
        let child = std::process::Command::new("/bin/true")
            .spawn()
            .expect("child");
        let pid = child.id();
        let zombie =
            || fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| s.contains(") Z "));
        while_idle(|| thread::sleep(INTERVAL * 3));
        assert!(!zombie(), "the exited child was left a zombie while idle");
    }
}
