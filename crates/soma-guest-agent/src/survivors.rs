//! The opt-in exception to "nothing survives a command".
//!
//! The sweep that ends every command kills each process in the guest. A command that started a
//! process on purpose to outlive it (the live browser is the first) names it in
//! [`SURVIVORS_FILE`], and the sweep that ends a command spares it. Shutdown and the pre-capture
//! sweep call [`crate::descendants::sweep_strays`] directly and never spare anything.

#![allow(unsafe_code)]

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use crate::descendants::{other_pids, sweep_strays};
use crate::pid1;

const INIT_PID: i32 = 1;
/// Where a command names the processes that are meant to outlive it, one decimal id per line.
///
/// An id matches a process when it is that process's id, its process group, or its session, so
/// naming the leader of a `setsid` session spares everything the leader started. The file lives
/// in `/run`, which every Instance gets as a fresh tmpfs, so a restored clone never inherits a
/// survivor list from the machine that was captured.
pub const SURVIVORS_FILE: &str = "/run/soma/survivors";
/// Longest the sweep waits for the processes it killed to leave the process table.
const SWEEP_BUDGET: Duration = Duration::from_secs(5);
const SWEEP_POLL: Duration = Duration::from_millis(1);

/// As PID 1, kills every other process except the survivors named in `survivors` and reaps the
/// ones it killed.
///
/// The ending of a command uses this instead of [`sweep_strays`]. It cannot reap with a blocking
/// `waitpid(-1)`, because a spared process is still a child of init and that call would then wait
/// for it to exit; it reaps without blocking until every process it signalled has left `/proc`.
///
/// Outside PID 1 this is a no-op so host tests never signal unrelated processes.
pub fn sweep_strays_sparing(survivors: &str) -> usize {
    if !pid1::is_pid1() {
        return 0;
    }
    let text = fs::read_to_string(survivors).unwrap_or_default();
    let ids = survivor_ids(&text);
    let cgroups = spared_cgroups(&text);
    if ids.is_empty() && cgroups.is_empty() {
        return sweep_strays();
    }
    let targets: Vec<i32> = doomed(&ids, &process_groups())
        .into_iter()
        .filter(|pid| {
            cgroups.is_empty()
                || !fs::read_to_string(format!("/proc/{pid}/cgroup"))
                    .is_ok_and(|contents| cgroup_spared(&cgroups, &contents))
        })
        .collect();
    for pid in &targets {
        // SAFETY: `kill` with a positive identifier has no memory preconditions; the identifier
        // was read from `/proc` and is neither this process nor a reserved value.
        unsafe { libc::kill(*pid, libc::SIGKILL) };
    }
    let deadline = Instant::now() + SWEEP_BUDGET;
    let mut reaped = 0;
    loop {
        loop {
            let mut status = 0;
            // SAFETY: `waitpid` writes one integer into the valid local; `WNOHANG` returns at
            // once, so a surviving child cannot hold this call.
            let pid = unsafe { libc::waitpid(-1, &raw mut status, libc::WNOHANG) };
            if pid > 0 {
                reaped += 1;
            } else {
                break;
            }
        }
        let remaining = targets
            .iter()
            .any(|pid| fs::metadata(format!("/proc/{pid}")).is_ok());
        if !remaining || Instant::now() >= deadline {
            return reaped;
        }
        thread::sleep(SWEEP_POLL);
    }
}

/// Parses the survivor list: one positive decimal id per line, anything else ignored.
///
/// Ids at or below init are dropped, so a list can never name init or exempt "everything".
#[must_use]
pub fn survivor_ids(text: &str) -> Vec<i32> {
    text.lines()
        .filter_map(|line| line.trim().parse::<i32>().ok())
        .filter(|id| *id > INIT_PID)
        .collect()
}

/// The cgroups a survivor list spares, from lines of the form `cgroup:/path`.
///
/// A process is spared when its cgroup is the named one or lies beneath it, which is how a
/// container engine survives the sweep that ends a command: the engine, its supervisor, every
/// shim, and every container process share two subtrees, and none of them can be named by process
/// id, group, or session, because containers start their own sessions. The root cgroup is never
/// accepted, so a list cannot exempt everything.
#[must_use]
pub fn spared_cgroups(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("cgroup:"))
        .map(|path| path.trim().trim_end_matches('/').to_owned())
        .filter(|path| path.starts_with('/') && path.len() > 1 && !path.contains(".."))
        .collect()
}

/// Whether the contents of `/proc/<pid>/cgroup` place the process in or under a spared cgroup.
#[must_use]
pub fn cgroup_spared(spared: &[String], proc_cgroup: &str) -> bool {
    // On cgroup v2 the file is the single line `0::/path`.
    proc_cgroup
        .lines()
        .filter_map(|line| line.strip_prefix("0::"))
        .any(|path| {
            spared
                .iter()
                .any(|root| path == root || path.starts_with(&format!("{root}/")))
        })
}

/// A process, its process group, and its session, as `/proc/<pid>/stat` reports them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Membership {
    /// The process identifier.
    pub pid: i32,
    /// The process group identifier.
    pub group: i32,
    /// The session identifier.
    pub session: i32,
}

/// Reads the group and session out of one `/proc/<pid>/stat` line.
///
/// The command name sits in parentheses and may itself contain spaces and parentheses, so the
/// fields are counted from the last closing parenthesis.
#[must_use]
pub fn parse_membership(pid: i32, stat: &str) -> Option<Membership> {
    let rest = &stat[stat.rfind(')')? + 1..];
    // After the name: state, parent, group, session.
    let mut fields = rest.split_whitespace().skip(2);
    let group = fields.next()?.parse().ok()?;
    let session = fields.next()?.parse().ok()?;
    Some(Membership {
        pid,
        group,
        session,
    })
}

/// The processes a sweep must kill: everything whose id, group, and session the list omits.
#[must_use]
pub fn doomed(survivors: &[i32], processes: &[Membership]) -> Vec<i32> {
    processes
        .iter()
        .filter(|process| {
            ![process.pid, process.group, process.session]
                .iter()
                .any(|id| survivors.contains(id))
        })
        .map(|process| process.pid)
        .collect()
}

fn process_groups() -> Vec<Membership> {
    other_pids()
        .into_iter()
        .filter_map(|pid| {
            // A process that exits between the listing and the read is already gone.
            let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            parse_membership(pid, &stat)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn survivor_lists_ignore_noise_and_never_name_init() {
        assert_eq!(
            survivor_ids("412\n 77 \nnot-a-number\n\n1\n0\n-5\n"),
            [412, 77]
        );
        assert!(survivor_ids("").is_empty());
    }

    #[test]
    fn group_and_session_come_from_after_the_last_parenthesis() {
        let stat = "412 (node (v22) x) S 1 412 412 0 -1 4194560 100";
        assert_eq!(
            parse_membership(412, stat),
            Some(Membership {
                pid: 412,
                group: 412,
                session: 412
            })
        );
        assert_eq!(parse_membership(9, "9 (short) S 1"), None);
        assert_eq!(parse_membership(9, "garbage"), None);
    }

    #[test]
    fn a_sweep_spares_the_listed_process_its_group_and_its_session_and_nothing_else() {
        let member = |pid, group, session| Membership {
            pid,
            group,
            session,
        };
        let processes = [
            member(412, 412, 412), // the daemon, a session leader
            member(430, 430, 412), // chromium, started by the daemon
            member(431, 440, 412), // a renderer in its own group, same session
            member(500, 500, 500), // a stray from the command
            member(501, 500, 500),
        ];
        assert_eq!(doomed(&[412], &processes), [500, 501]);
        assert_eq!(doomed(&[], &processes), [412, 430, 431, 500, 501]);
        // An id that matches nothing spares nothing.
        assert_eq!(doomed(&[999], &processes).len(), 5);
    }

    #[test]
    fn cgroup_lines_spare_the_named_subtree_and_never_the_root() {
        let text = "412\ncgroup:/docker-engine\n cgroup:/docker/ \ncgroup:/\ncgroup:relative\ncgroup:/a/../b\n";
        assert_eq!(spared_cgroups(text), ["/docker-engine", "/docker"]);
        // Plain id lines and cgroup lines do not interfere.
        assert_eq!(survivor_ids(text), [412]);
        let spared = spared_cgroups(text);
        assert!(cgroup_spared(&spared, "0::/docker-engine\n"));
        assert!(cgroup_spared(&spared, "0::/docker/3f9c\n"));
        assert!(cgroup_spared(&spared, "0::/docker/3f9c/sub\n"));
        // A sibling that merely shares a prefix is not under the named cgroup.
        assert!(!cgroup_spared(&spared, "0::/docker-other\n"));
        assert!(!cgroup_spared(&spared, "0::/\n"));
        assert!(!cgroup_spared(&spared, ""));
        assert!(!cgroup_spared(&[], "0::/docker\n"));
    }

    #[test]
    fn the_sparing_sweep_is_inert_outside_pid_one() {
        assert_eq!(sweep_strays_sparing(SURVIVORS_FILE), 0);
    }

    // The sweep under test is what reaps these children, as init does for any orphan.
    #[allow(clippy::zombie_processes)]
    #[test]
    fn as_pid_one_the_sparing_sweep_keeps_the_survivor_and_kills_the_stray() {
        // Only meaningful as init, which a test run is not; the container run in the
        // browser-default runbook starts this binary as PID 1 to exercise it.
        if !pid1::is_pid1() {
            return;
        }
        let survivor = std::process::Command::new("/usr/bin/setsid")
            .args(["/bin/sleep", "60"])
            .spawn()
            .expect("survivor");
        let stray = std::process::Command::new("/bin/sleep")
            .arg("60")
            .spawn()
            .expect("stray");
        // `setsid` execs `sleep` in place when it is not a group leader, so the child's id is
        // the session id either way.
        let list = std::env::temp_dir().join("soma-survivors-test");
        fs::write(&list, format!("{}\n", survivor.id())).expect("list");
        thread::sleep(Duration::from_millis(100));

        sweep_strays_sparing(list.to_str().expect("utf8 path"));

        let alive = |pid: u32| {
            fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| !stat.contains(") Z "))
        };
        assert!(alive(survivor.id()), "the listed survivor was killed");
        assert!(!alive(stray.id()), "the stray outlived the sweep");
        // SAFETY: killing the one child this test started.
        unsafe { libc::kill(i32::try_from(survivor.id()).expect("pid"), libc::SIGKILL) };
    }
}
