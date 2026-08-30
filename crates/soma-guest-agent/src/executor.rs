//! Bounded direct command execution: argv `execve`, fixed environment, own process group,
//! one bounded poll loop over both pipes with exact accounting, absolute deadline, and
//! complete descendant reaping.
//!
//! No output is queued and no reader thread exists, so a hostile process that writes without
//! end cannot grow the agent beyond one fixed buffer plus one admitted chunk.

use std::iter;
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use soma_guest::{GuestCommand, TerminalStatus};

use crate::descendants;
use crate::environment::{ENVIRONMENT, InvalidInvocation, Invocation, WORKING_DIRECTORY};
use crate::output::{Ending, OutputBudget, terminal_status};
use crate::pid1;
use crate::timings::{self, Step};

mod pipes;

/// Time the killed process group is given to die while its pipes are drained.
pub const KILL_GRACE: Duration = Duration::from_millis(500);
/// Resident bytes one command may cost the agent beyond its fixed stacks and the sink.
///
/// The bounded loop owns one `MAX_CHUNK_BYTES` read buffer and hands the sink one borrowed
/// prefix of that same buffer, so nothing scales with the volume a child produces.
pub const RESIDENT_OUTPUT_BYTES: usize = crate::output::MAX_CHUNK_BYTES;

/// First wait between reapability checks after the child's pipes reached their end.
///
/// A child that closed its pipes is already inside its own exit and becomes reapable within
/// microseconds, so the first check must not cost more than that; the flat ceiling below was
/// the whole of the readiness probe whenever the parent lost that race.
const FIRST_WAIT_POLL: Duration = Duration::from_micros(50);
/// Longest wait between reapability checks, for a child that outlives its own pipes.
const WAIT_POLL: Duration = Duration::from_millis(5);

/// Destination for admitted output chunks.
///
/// The slice borrows the executor's fixed buffer and is valid only for the call.
pub trait OutputSink {
    /// Delivers one nonempty admitted stdout chunk.
    ///
    /// # Errors
    ///
    /// Returns [`SinkFault`] if the chunk could not be delivered; execution is then aborted.
    fn stdout(&mut self, bytes: &[u8]) -> Result<(), SinkFault>;
    /// Delivers one nonempty admitted stderr chunk.
    ///
    /// # Errors
    ///
    /// Returns [`SinkFault`] if the chunk could not be delivered; execution is then aborted.
    fn stderr(&mut self, bytes: &[u8]) -> Result<(), SinkFault>;
}

/// The sink rejected output and the command must be aborted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SinkFault;

/// Redacted executor failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutorFault {
    /// The command violated a local bound.
    Invocation(InvalidInvocation),
    /// The output sink failed; the process group was killed and reaped.
    Sink,
    /// The child could not be identified as a process group leader.
    ProcessGroup,
}

/// Result of one bounded execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Completion {
    /// The protocol terminal status.
    pub status: TerminalStatus,
    /// The process group that was created, killed, and reaped.
    pub process_group: i32,
    /// Stdout bytes delivered to the sink.
    pub stdout_bytes: u64,
    /// Stderr bytes delivered to the sink.
    pub stderr_bytes: u64,
}

/// Executes one command to completion, streaming admitted output into the sink.
///
/// # Errors
///
/// Returns an executor fault; process results including `execve` failure are ordinary statuses.
pub fn execute(
    command: &GuestCommand,
    sink: &mut impl OutputSink,
) -> Result<Completion, ExecutorFault> {
    let invocation = Invocation::from_command(command).map_err(ExecutorFault::Invocation)?;
    let deadline = Instant::now() + Duration::from_millis(u64::from(command.timeout_millis()));
    let mut child = match timings::measure(Step::Spawn, || spawn(&invocation)) {
        Ok(child) => child,
        Err(errno) => {
            return Ok(Completion {
                status: terminal_status(false, false, Ending::ExecFailed(errno)),
                process_group: 0,
                stdout_bytes: 0,
                stderr_bytes: 0,
            });
        }
    };
    let process_group = i32::try_from(child.id()).unwrap_or(0);
    if process_group <= 1 {
        let _ = child.kill();
        let _ = child.wait();
        return Err(ExecutorFault::ProcessGroup);
    }
    let mut budget = OutputBudget::new(command.output_bytes());
    let streamed = timings::measure(Step::Stream, || {
        pipes::stream(
            child.stdout.take(),
            child.stderr.take(),
            &mut budget,
            sink,
            deadline,
            process_group,
        )
    });
    let ending = timings::measure(Step::Wait, || {
        wait_for_child(&mut child, streamed.kill_by, process_group)
    });
    timings::measure(Step::Reap, || {
        descendants::kill_group(process_group);
        descendants::reap_group(process_group);
        pid1::reap_orphans();
        descendants::sweep_strays();
    });
    if streamed.sink_failed {
        return Err(ExecutorFault::Sink);
    }
    Ok(Completion {
        status: terminal_status(streamed.limit_hit, streamed.timed_out, ending),
        process_group,
        stdout_bytes: streamed.stdout_bytes,
        stderr_bytes: streamed.stderr_bytes,
    })
}

fn spawn(invocation: &Invocation) -> Result<Child, i32> {
    Command::new(invocation.program())
        .args(invocation.arguments())
        .env_clear()
        .envs(ENVIRONMENT.iter().copied())
        .current_dir(WORKING_DIRECTORY)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|error| error.raw_os_error().unwrap_or(libc::EIO))
}

/// The waits between reapability checks, in the order the loop takes them.
///
/// The sequence starts at [`FIRST_WAIT_POLL`], doubles, and saturates at [`WAIT_POLL`]; it is
/// endless, so the loop's absolute deadline is what ends the wait.
fn waits() -> impl Iterator<Item = Duration> {
    iter::successors(Some(FIRST_WAIT_POLL), |poll| {
        Some(poll.saturating_mul(2).min(WAIT_POLL))
    })
}

fn wait_for_child(child: &mut Child, until: Instant, process_group: i32) -> Ending {
    let mut waits = waits();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return ending(status),
            Ok(None) if Instant::now() < until => {
                thread::sleep(waits.next().unwrap_or(WAIT_POLL));
            }
            Ok(None) => {
                descendants::kill_group(process_group);
                return child.wait().map_or(Ending::Unknown, ending);
            }
            Err(_) => return Ending::Unknown,
        }
    }
}

fn ending(status: ExitStatus) -> Ending {
    if let Some(code) = status.code() {
        return Ending::Exited(code);
    }
    status
        .signal()
        .and_then(|signal| u8::try_from(signal).ok())
        .map_or(Ending::Unknown, Ending::Signaled)
}

#[cfg(test)]
mod tests;
