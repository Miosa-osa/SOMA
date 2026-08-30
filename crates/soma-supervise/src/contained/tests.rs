//! Containment gates for one external tool.
//!
//! The fixtures are ordinary shell programs, so they exercise the real spawn, group, deadline,
//! overflow, termination, and collection paths rather than a stub.

#![cfg(unix)]

use std::{
    fs,
    process::Command,
    time::{Duration, Instant},
};

use super::{Contained, Output, TERMINATION_GRACE, Uncontained};
use crate::CAPTURE_LIMIT;

const DEADLINE: Duration = Duration::from_millis(300);

fn shell(script: &str, deadline: Duration) -> Result<Output, Uncontained<()>> {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", script]);
    Contained::new(command, deadline).run(|_| Ok(()))
}

/// Waits, bounded, for a recorded process identifier to disappear from `/proc`.
#[cfg(target_os = "linux")]
fn process_is_gone(pid_file: &std::path::Path) -> bool {
    use std::path::Path;

    let text = fs::read_to_string(pid_file).expect("the fixture recorded its descendant");
    let pid: u32 = text.trim().parse().expect("a numeric process identifier");
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if !Path::new(&format!("/proc/{pid}")).exists() {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn scratch(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!("soma-supervise-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("scratch directory");
    directory.join(name)
}

#[test]
fn a_bounded_tool_reports_its_exit_code_and_both_streams() {
    let output =
        shell("echo out; echo err >&2; exit 7", Duration::from_secs(30)).expect("a bounded tool");

    assert_eq!(output.exit_code, Some(7));
    assert_eq!(output.stdout, b"out\n");
    assert_eq!(output.stderr, b"err\n");
    assert!(!output.succeeded());
    assert!(
        shell("exit 0", Duration::from_secs(30))
            .expect("zero")
            .succeeded()
    );
}

#[test]
fn a_feed_failure_terminates_the_group_and_returns_the_caller_error() {
    let started = Instant::now();
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "exec /bin/sleep 300"]);
    let failure = Contained::new(command, Duration::from_secs(120))
        .run(|_| Err("feed refused"))
        .expect_err("feed failure");

    assert_eq!(failure, Uncontained::Input("feed refused"));
    assert!(
        started.elapsed() <= TERMINATION_GRACE + Duration::from_secs(1),
        "a feed failure must not wait for the tool deadline"
    );
}

#[test]
fn an_input_write_to_a_tool_that_never_reads_is_a_caller_failure() {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "exit 0"]);
    let payload = vec![b'x'; 4 * 1024 * 1024];
    let failure = Contained::new(command, Duration::from_secs(30))
        .run(|stdin| stdin.write_all(&payload).map_err(|_| "write refused"))
        .expect_err("the tool closed its read end");

    assert_eq!(failure, Uncontained::Input("write refused"));
}

#[test]
fn a_stream_that_exceeds_the_capture_ceiling_terminates_the_group() {
    let line = "a".repeat(4096);
    let started = Instant::now();
    let failure = shell(
        &format!("while : ; do echo {line}; done"),
        Duration::from_secs(120),
    )
    .expect_err("a flooding tool");

    assert_eq!(failure, Uncontained::Terminated);
    assert!(
        started.elapsed() <= TERMINATION_GRACE + Duration::from_secs(5),
        "an overflowing tool must be terminated rather than drained to its deadline"
    );
    assert_eq!(CAPTURE_LIMIT, 64 * 1024);
}

#[test]
fn a_tool_that_ignores_the_polite_signal_is_forced_after_the_grace() {
    let pid_file = scratch("stubborn.pid");
    let _ = fs::remove_file(&pid_file);
    let script = format!(
        "trap '' TERM; /bin/sleep 300 & echo $! > {}; while : ; do /bin/sleep 1; done",
        pid_file.to_string_lossy()
    );
    let started = Instant::now();

    let failure = shell(&script, DEADLINE).expect_err("stubborn tool");

    assert_eq!(failure, Uncontained::Terminated);
    assert!(
        started.elapsed() <= DEADLINE + TERMINATION_GRACE,
        "the invocation took {:?}, beyond the deadline plus the declared grace",
        started.elapsed()
    );
    #[cfg(target_os = "linux")]
    assert!(
        process_is_gone(&pid_file),
        "a group member survived the force signal"
    );
}

#[test]
fn a_descendant_holding_both_pipes_cannot_outlive_the_invocation() {
    let pid_file = scratch("descendant.pid");
    let _ = fs::remove_file(&pid_file);
    let script = format!(
        "/bin/sleep 300 & echo $! > {}; exit 0",
        pid_file.to_string_lossy()
    );
    let started = Instant::now();

    let output = shell(&script, DEADLINE).expect("the leader exited on its own");

    assert_eq!(output.exit_code, Some(0));
    assert!(
        started.elapsed() <= DEADLINE + TERMINATION_GRACE,
        "the invocation took {:?}, beyond the deadline plus the declared grace",
        started.elapsed()
    );
    #[cfg(target_os = "linux")]
    assert!(
        process_is_gone(&pid_file),
        "a descendant survived the invocation that forked it"
    );
}

/// The leader exits zero at once while a descendant keeps only the standard-input read end and
/// never reads it, so nothing but the group signal can unblock the caller's feed.
#[test]
fn a_descendant_holding_the_input_pipe_cannot_block_the_feed() {
    let pid_file = scratch("input-holder.pid");
    let _ = fs::remove_file(&pid_file);
    let script = format!(
        "exec 3<&0; {{ /bin/sleep 300; }} <&3 & echo $! > {}; exit 0",
        pid_file.to_string_lossy()
    );
    let (report, outcome) = std::sync::mpsc::channel();
    let started = Instant::now();
    let feeder = std::thread::spawn(move || {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", &script]);
        let payload = vec![b'x'; 4 * 1024 * 1024];
        let result = Contained::new(command, Duration::from_secs(120))
            .run(|stdin| stdin.write_all(&payload).map_err(|_| "write refused"));
        let _ = report.send(());
        result
    });

    outcome
        .recv_timeout(Duration::from_secs(30))
        .expect("the invocation must end even though a descendant holds the input pipe");
    let failure = feeder
        .join()
        .expect("the feeding thread")
        .expect_err("the descendant never read the payload");

    assert_eq!(failure, Uncontained::Input("write refused"));
    assert!(
        started.elapsed() <= TERMINATION_GRACE + Duration::from_secs(5),
        "the invocation took {:?}, so the feed was not bounded",
        started.elapsed()
    );
    #[cfg(target_os = "linux")]
    assert!(
        process_is_gone(&pid_file),
        "a descendant holding the input pipe survived the invocation"
    );
}

#[test]
fn a_program_that_cannot_be_started_is_a_spawn_failure() {
    let command = Command::new(scratch("no-such-tool"));
    let failure = Contained::new(command, DEADLINE)
        .run(|_| Ok::<(), ()>(()))
        .expect_err("missing program");

    assert_eq!(failure, Uncontained::Spawn);
}
