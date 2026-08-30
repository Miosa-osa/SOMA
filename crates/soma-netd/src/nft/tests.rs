//! Containment gates for the broker's privileged tool seam.
//!
//! The fixtures are ordinary shell programs driven through the exact [`Invocation`] the
//! production `nft` and `conntrack` calls use, so a wedged tool is proved to end in a typed
//! failure inside the declared bound rather than to hang the single-threaded broker.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use soma_supervise::{CAPTURE_LIMIT, TERMINATION_GRACE};

use super::{DEADLINE, Invocation};
use crate::{Error, Tool};

const SHELL: &str = "/bin/sh";
const TEST_DEADLINE: Duration = Duration::from_millis(300);

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("soma-netd-tools-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("scratch directory");
    directory.join(name)
}

fn shell<'a>(script: &'a [&'a str], deadline: Duration) -> Invocation<'a> {
    Invocation {
        tool: Tool::Nft,
        program: SHELL,
        arguments: script,
        deadline,
    }
}

/// Waits, bounded, for a recorded process identifier to disappear from `/proc`.
fn process_is_gone(pid_file: &Path) -> bool {
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

#[test]
fn a_bounded_tool_reports_its_status_and_the_output_the_parsers_read() {
    let output = shell(&["-c", "echo table inet soma-x; exit 0"], DEADLINE)
        .run("")
        .expect("a bounded tool");

    assert!(output.succeeded());
    assert_eq!(output.stdout, b"table inet soma-x\n");

    let failed = shell(&["-c", "exit 3"], DEADLINE)
        .run("")
        .expect("a bounded tool");
    assert_eq!(failed.exit_code, Some(3));
    assert!(!failed.succeeded());
}

#[test]
fn a_tool_that_ignores_the_polite_signal_cannot_outlive_its_deadline() {
    let pid_file = scratch("stubborn.pid");
    let _ = fs::remove_file(&pid_file);
    let script = format!(
        "trap '' TERM; /bin/sleep 300 & echo $! > {}; while : ; do /bin/sleep 1; done",
        pid_file.to_string_lossy()
    );
    let started = Instant::now();

    let error = shell(&["-c", &script], TEST_DEADLINE)
        .run("")
        .expect_err("a wedged tool");

    assert_eq!(
        error,
        Error::Tool {
            tool: Tool::Nft,
            status: None
        }
    );
    assert!(
        started.elapsed() <= TEST_DEADLINE + TERMINATION_GRACE,
        "the broker waited {:?}, beyond the deadline plus the declared grace",
        started.elapsed()
    );
    assert!(
        process_is_gone(&pid_file),
        "a group member survived the force signal"
    );
}

#[test]
fn a_descendant_holding_the_pipes_cannot_wedge_the_broker() {
    let pid_file = scratch("descendant.pid");
    let _ = fs::remove_file(&pid_file);
    let script = format!(
        "/bin/sleep 300 & echo $! > {}; exit 0",
        pid_file.to_string_lossy()
    );
    let started = Instant::now();

    let error = shell(&["-c", &script], TEST_DEADLINE)
        .run("")
        .expect_err("a stray descendant");

    assert_eq!(
        error,
        Error::Tool {
            tool: Tool::Nft,
            status: None
        }
    );
    assert!(
        started.elapsed() <= TEST_DEADLINE + TERMINATION_GRACE,
        "the broker waited {:?}, beyond the deadline plus the declared grace",
        started.elapsed()
    );
    assert!(
        process_is_gone(&pid_file),
        "a descendant survived a failed invocation"
    );
}

#[test]
fn a_ruleset_the_tool_refuses_to_read_is_the_operations_failure() {
    let ruleset = "add rule inet soma filter drop\n".repeat(200_000);
    assert!(ruleset.len() > 4 * 1024 * 1024);

    let error = shell(&["-c", "exit 0"], DEADLINE)
        .run(&ruleset)
        .expect_err("the tool closed its read end");

    assert_eq!(
        error,
        Error::Tool {
            tool: Tool::Nft,
            status: None
        }
    );
}

#[test]
fn a_tool_that_floods_its_output_is_terminated_rather_than_buffered() {
    let line = "table inet soma-flood".repeat(200);
    let started = Instant::now();

    let error = shell(
        &["-c", &format!("while : ; do echo {line}; done")],
        DEADLINE,
    )
    .run("")
    .expect_err("a flooding tool");

    assert_eq!(
        error,
        Error::Tool {
            tool: Tool::Nft,
            status: None
        }
    );
    assert!(
        started.elapsed() < DEADLINE,
        "the flood was drained to the deadline instead of terminating the group"
    );
    assert_eq!(CAPTURE_LIMIT, 64 * 1024);
}

#[test]
fn an_absent_tool_is_a_typed_failure_rather_than_a_panic() {
    let missing = scratch("no-such-tool");
    let _ = fs::remove_file(&missing);
    let path = missing.to_string_lossy().into_owned();

    let error = Invocation {
        tool: Tool::Conntrack,
        program: &path,
        arguments: &[],
        deadline: DEADLINE,
    }
    .run("")
    .expect_err("a missing tool");

    assert_eq!(
        error,
        Error::Tool {
            tool: Tool::Conntrack,
            status: None
        }
    );
}
