//! The exec policy: what a command leaves behind outlives it only when the Instance keeps
//! leftovers and the command ended on its own.

use std::fs;

use soma_guest::TerminalStatus;

use super::{RecordingSink, command, group_is_gone};
use crate::executor::execute_under;

/// A server started in the background, its output away from the command's pipes.
const BACKGROUND_SERVER: &str = "/bin/sleep 30 >/dev/null 2>&1 & echo started";

fn run_under(
    policy: &str,
    program: &str,
    arguments: &[&str],
    timeout_millis: u32,
) -> (TerminalStatus, i32) {
    let directory = tempfile::tempdir().expect("a directory for the policy");
    let path = directory.path().join("exec-policy");
    fs::write(&path, policy).expect("policy");
    let mut sink = RecordingSink::default();
    let completion = execute_under(
        &command(program, arguments, timeout_millis, 4_096),
        &mut sink,
        path.to_str().expect("utf8 path"),
    )
    .expect("executor");
    (completion.status, completion.process_group)
}

fn group_is_alive(group: i32) -> bool {
    // SAFETY: signal zero performs only a permission and existence check.
    unsafe { libc::kill(-group, 0) == 0 }
}

fn end_group(group: i32) {
    // SAFETY: killing the one process group this test started.
    unsafe { libc::kill(-group, libc::SIGKILL) };
}

#[test]
fn a_kept_session_outlives_a_command_that_ended_on_its_own() {
    let (status, group) = run_under(
        "keep-session\n",
        "/bin/sh",
        &["-c", BACKGROUND_SERVER],
        5_000,
    );

    assert_eq!(status, TerminalStatus::Exited(0));
    assert!(
        group_is_alive(group),
        "the background server ended with its command"
    );
    end_group(group);
}

#[test]
fn without_the_policy_the_background_server_ends_with_its_command() {
    let (status, group) = run_under("", "/bin/sh", &["-c", BACKGROUND_SERVER], 5_000);

    assert_eq!(status, TerminalStatus::Exited(0));
    assert!(
        group_is_gone(group),
        "the background server outlived its command"
    );
}

#[test]
fn a_kept_session_still_ends_at_the_deadline() {
    let (status, group) = run_under(
        "keep-session\n",
        "/bin/sh",
        &["-c", "/bin/sleep 30 >/dev/null 2>&1 & /bin/sleep 30"],
        200,
    );

    assert_eq!(status, TerminalStatus::TimedOut);
    assert!(
        group_is_gone(group),
        "a command past its deadline left a process behind"
    );
}
