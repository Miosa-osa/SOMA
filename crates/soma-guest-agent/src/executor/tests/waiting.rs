//! The wait after a child's pipes closed: woken by the exit, and still bounded by the deadline.

use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use soma_guest::TerminalStatus;

use super::{command, group_is_gone, run};
use crate::executor::exit::{self, Watched};
use crate::executor::poll_for_child;
use crate::output::Ending;

/// A child in its own process group with nothing attached to its standard streams.
fn leader(program: &str, arguments: &[&str]) -> (Child, i32) {
    let child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .expect("a test child");
    let group = i32::try_from(child.id()).expect("a process identifier");
    (child, group)
}

#[test]
fn the_watch_reports_an_exit_and_the_child_is_then_reaped_with_its_status() {
    let (mut child, _) = leader("/bin/sh", &["-c", "exit 7"]);

    assert_eq!(
        exit::watch(&child, Instant::now() + Duration::from_secs(5)),
        Watched::Exited
    );
    // Readable means exited, so collecting the status cannot block.
    let status = child.try_wait().expect("a wait").expect("an exited child");
    assert_eq!(status.code(), Some(7));
}

#[test]
fn the_watch_gives_up_at_the_deadline_and_leaves_the_kill_to_the_caller() {
    let (mut child, group) = leader("/bin/sleep", &["30"]);
    let started = Instant::now();

    assert_eq!(
        exit::watch(&child, started + Duration::from_millis(200)),
        Watched::Deadline
    );
    assert!(started.elapsed() >= Duration::from_millis(200));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(
        child.try_wait().expect("a wait").is_none(),
        "the watch only waits; it neither signals nor reaps"
    );
    crate::descendants::kill_group(group);
    let _ = child.wait();
}

#[test]
fn a_child_that_outlives_its_pipes_is_killed_at_the_deadline() {
    let started = Instant::now();
    let (completion, _) = run(&command(
        "/bin/sh",
        &["-c", "exec >/dev/null 2>&1; exec /bin/sleep 30"],
        1_500,
        1,
    ));

    // The shell closes the pipes within milliseconds, far inside the deadline, so the deadline
    // lands in the wait, which kills the group and reports the kill as the ending.
    assert_eq!(completion.status, TerminalStatus::Signaled(9));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(group_is_gone(completion.process_group));
}

#[test]
fn the_sleep_schedule_still_bounds_a_wait_the_kernel_will_not_watch() {
    let (mut exiting, group) = leader("/bin/true", &[]);
    assert_eq!(
        poll_for_child(&mut exiting, Instant::now() + Duration::from_secs(5), group),
        Ending::Exited(0)
    );

    let (mut running, group) = leader("/bin/sleep", &["30"]);
    let started = Instant::now();
    assert_eq!(
        poll_for_child(&mut running, started + Duration::from_millis(200), group),
        Ending::Signaled(9)
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}
