#![allow(unsafe_code)]

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use soma_guest::{GuestCommand, TerminalStatus};

use super::{Completion, ExecutorFault, KILL_GRACE, OutputSink, SinkFault, execute};

mod hostile;

/// Combined allowance the hostile fixtures are given before the limit closes them down.
const HOSTILE_ALLOWANCE: u64 = 8 * 1024 * 1024;
/// Declared resident growth one hostile command may cost the agent.
///
/// The bounded loop owns one fixed read buffer and one borrowed chunk, so the real growth is
/// far below this ceiling; an unbounded queue behind a slow sink exceeds it within a second.
const MAX_RESIDENT_GROWTH_BYTES: u64 = 32 * 1024 * 1024;
/// Two descendants of one shell, each holding one pipe and writing at memory speed.
const HOSTILE_BOTH_PIPES: &str = "/bin/cat /dev/zero & /bin/cat /dev/zero >&2";

#[derive(Default)]
struct RecordingSink {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    fail_after: Option<usize>,
    chunks: usize,
}

impl OutputSink for RecordingSink {
    fn stdout(&mut self, bytes: &[u8]) -> Result<(), SinkFault> {
        self.chunks += 1;
        if self.fail_after.is_some_and(|limit| self.chunks > limit) {
            return Err(SinkFault);
        }
        self.stdout.extend_from_slice(bytes);
        Ok(())
    }

    fn stderr(&mut self, bytes: &[u8]) -> Result<(), SinkFault> {
        self.chunks += 1;
        if self.fail_after.is_some_and(|limit| self.chunks > limit) {
            return Err(SinkFault);
        }
        self.stderr.extend_from_slice(bytes);
        Ok(())
    }
}

/// Counts and paces delivery without retaining a byte, so only the executor bounds memory.
struct SlowCountingSink {
    stdout: u64,
    stderr: u64,
    largest_chunk: usize,
    delay: Duration,
}

impl SlowCountingSink {
    const fn new(delay: Duration) -> Self {
        Self {
            stdout: 0,
            stderr: 0,
            largest_chunk: 0,
            delay,
        }
    }

    fn count(&mut self, bytes: &[u8]) {
        self.largest_chunk = self.largest_chunk.max(bytes.len());
        thread::sleep(self.delay);
    }
}

impl OutputSink for SlowCountingSink {
    fn stdout(&mut self, bytes: &[u8]) -> Result<(), SinkFault> {
        self.count(bytes);
        self.stdout += bytes.len() as u64;
        Ok(())
    }

    fn stderr(&mut self, bytes: &[u8]) -> Result<(), SinkFault> {
        self.count(bytes);
        self.stderr += bytes.len() as u64;
        Ok(())
    }
}

fn command(program: &str, arguments: &[&str], timeout_millis: u32, output: u64) -> GuestCommand {
    GuestCommand::new(
        program.as_bytes().to_vec(),
        arguments
            .iter()
            .map(|argument| argument.as_bytes().to_vec())
            .collect(),
        timeout_millis,
        output,
    )
    .expect("bounded command")
}

fn run(command: &GuestCommand) -> (Completion, RecordingSink) {
    let mut sink = RecordingSink::default();
    let completion = execute(command, &mut sink).expect("executor");
    (completion, sink)
}

fn group_is_gone(process_group: i32) -> bool {
    // Orphaned members are reaped by the host init, not by this test process, so allow the
    // host a bounded moment to collect zombies that still count as group members.
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        // SAFETY: signal zero performs only a permission and existence check.
        let result = unsafe { libc::kill(-process_group, 0) };
        if result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Peak resident set size of this process in bytes, as the kernel reports it.
fn resident_high_water() -> u64 {
    fs::read_to_string("/proc/self/status")
        .expect("process status")
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
        .expect("VmHWM in kibibytes")
        * 1024
}

#[test]
fn a_successful_command_exits_zero_without_output() {
    let (completion, sink) = run(&command("/bin/true", &[], 5_000, 1));

    assert_eq!(completion.status, TerminalStatus::Exited(0));
    assert!(sink.stdout.is_empty() && sink.stderr.is_empty());
    assert_eq!((completion.stdout_bytes, completion.stderr_bytes), (0, 0));
    assert!(completion.process_group > 1);
    assert!(group_is_gone(completion.process_group));
}

#[test]
fn arguments_reach_the_program_verbatim_and_output_is_streamed() {
    let (completion, sink) = run(&command("/bin/echo", &["$HOME", "a b"], 5_000, 4096));

    assert_eq!(completion.status, TerminalStatus::Exited(0));
    assert_eq!(sink.stdout, b"$HOME a b\n");
    assert_eq!(completion.stdout_bytes, 10);
    assert_eq!(completion.stderr_bytes, 0);
}

#[test]
fn nonzero_exit_and_stderr_are_reported() {
    let (completion, sink) = run(&command(
        "/bin/ls",
        &["/soma-nonexistent-path"],
        5_000,
        4096,
    ));

    assert_eq!(completion.status, TerminalStatus::Exited(2));
    assert!(!sink.stderr.is_empty());
    assert!(sink.stdout.is_empty());
    assert_eq!(completion.stderr_bytes, sink.stderr.len() as u64);
    assert_eq!(completion.stdout_bytes, 0);
}

#[test]
fn a_missing_program_is_an_exec_failure_with_errno() {
    let (completion, _) = run(&command("/soma/no/such/program", &[], 5_000, 1));

    assert_eq!(completion.status, TerminalStatus::ExecFailed(libc::ENOENT));
}

#[test]
fn the_deadline_kills_the_process_group_and_reports_timeout() {
    let started = Instant::now();
    let (completion, _) = run(&command("/bin/sleep", &["30"], 200, 1));

    assert_eq!(completion.status, TerminalStatus::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(group_is_gone(completion.process_group));
}

#[test]
fn background_descendants_die_with_the_process_group() {
    let started = Instant::now();
    let (completion, _) = run(&command(
        "/bin/sh",
        &["-c", "/bin/sleep 30 & /bin/sleep 30"],
        200,
        1,
    ));

    assert_eq!(completion.status, TerminalStatus::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(group_is_gone(completion.process_group));
}

#[test]
fn the_output_allowance_is_filled_exactly_then_the_command_is_killed() {
    let (completion, sink) = run(&command("/bin/cat", &["/dev/zero"], 5_000, 10_000));

    assert_eq!(completion.status, TerminalStatus::OutputLimit);
    assert_eq!(sink.stdout.len(), 10_000);
    assert_eq!(completion.stdout_bytes, 10_000);
    assert!(group_is_gone(completion.process_group));
}

#[test]
fn output_exactly_at_the_allowance_is_a_normal_exit() {
    let (completion, sink) = run(&command("/bin/echo", &["-n", "abcd"], 5_000, 4));

    assert_eq!(completion.status, TerminalStatus::Exited(0));
    assert_eq!(sink.stdout, b"abcd");
    assert_eq!(completion.stdout_bytes, 4);
}

#[test]
fn a_signal_death_is_reported_with_the_signal_number() {
    let (completion, _) = run(&command("/bin/sh", &["-c", "kill -9 $$"], 5_000, 1));

    assert_eq!(completion.status, TerminalStatus::Signaled(9));
}

#[test]
fn a_failing_sink_aborts_and_reaps_the_command() {
    let mut sink = RecordingSink {
        fail_after: Some(0),
        ..RecordingSink::default()
    };
    let started = Instant::now();
    let error = execute(
        &command("/bin/cat", &["/dev/zero"], 5_000, 1 << 20),
        &mut sink,
    )
    .expect_err("sink failure");

    assert_eq!(error, ExecutorFault::Sink);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_sink_that_disconnects_mid_stream_kills_the_group_within_the_grace() {
    let mut sink = RecordingSink {
        fail_after: Some(2),
        ..RecordingSink::default()
    };
    let started = Instant::now();
    let error = execute(
        &command("/bin/sh", &["-c", HOSTILE_BOTH_PIPES], 60_000, 1 << 24),
        &mut sink,
    )
    .expect_err("sink disconnect");

    assert_eq!(error, ExecutorFault::Sink);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a disconnect must complete within the kill grace, not the command timeout"
    );
    assert!(KILL_GRACE < Duration::from_secs(5));
}

#[test]
fn the_environment_is_the_fixed_allowlist_only() {
    let (completion, sink) = run(&command("/usr/bin/env", &[], 5_000, 4096));

    assert_eq!(completion.status, TerminalStatus::Exited(0));
    let text = String::from_utf8_lossy(&sink.stdout);
    assert!(text.contains("SOMA_SANDBOX=1\n"));
    assert!(text.contains("HOME=/root\n"));
    assert!(!text.contains("USER="));
    assert!(!text.contains("CARGO"));
}
