use std::{ffi::OsString, path::PathBuf, time::Duration};

use crate::{
    ExecutionStatus,
    process::{ProcessInvocation, ProcessRunner, SystemProcessRunner},
};

#[test]
fn system_runner_enforces_the_output_limit_during_ingress() {
    let invocation = ProcessInvocation::new(
        PathBuf::from("/usr/bin/yes"),
        Vec::new(),
        Duration::from_secs(2),
        32,
    );

    let output = SystemProcessRunner
        .run(&invocation)
        .expect("bounded reader terminates the process");

    assert_eq!(output.status(), ExecutionStatus::OutputLimitExceeded);
    let (_, stdout, stdout_observed, stderr, stderr_observed, _) = output.into_observed_parts();
    assert!(stdout.len() + stderr.len() <= 32);
    assert!(stdout_observed + stderr_observed > 32);
}

#[test]
fn system_runner_preserves_overflow_evidence_after_a_fast_process_exits() {
    let invocation = ProcessInvocation::new(
        PathBuf::from("/usr/bin/printf"),
        vec![OsString::from("abcdefgh")],
        Duration::from_secs(2),
        3,
    );

    let output = SystemProcessRunner
        .run(&invocation)
        .expect("short process output is captured");

    assert_eq!(output.status(), ExecutionStatus::OutputLimitExceeded);
    let (_, stdout, stdout_observed, stderr, stderr_observed, _) = output.into_observed_parts();
    assert_eq!(stdout.len() + stderr.len(), 3);
    assert_eq!(stdout_observed + stderr_observed, 8);
}

#[test]
fn system_runner_terminates_a_timed_out_process() {
    let invocation = ProcessInvocation::new(
        PathBuf::from("/bin/sleep"),
        vec![OsString::from("1")],
        Duration::from_millis(20),
        32,
    );

    let output = SystemProcessRunner
        .run(&invocation)
        .expect("timeout kills and reaps the process");

    assert_eq!(output.status(), ExecutionStatus::TimedOut);
}
