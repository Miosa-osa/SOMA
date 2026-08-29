//! Bounded direct command execution: argv `execve`, fixed environment, own process group,
//! one bounded poll loop over both pipes with exact accounting, absolute deadline, and
//! complete descendant reaping.
//!
//! No output is queued and no reader thread exists, so a hostile process that writes without
//! end cannot grow the agent beyond one fixed buffer plus one admitted chunk.

use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use soma_guest::{GuestCommand, TerminalStatus};

use crate::descendants;
use crate::environment::{ENVIRONMENT, InvalidInvocation, Invocation, WORKING_DIRECTORY};
use crate::output::{Ending, OutputBudget, terminal_status};
use crate::pid1;

mod pipes;

/// Time the killed process group is given to die while its pipes are drained.
pub const KILL_GRACE: Duration = Duration::from_millis(500);
/// Resident bytes one command may cost the agent beyond its fixed stacks and the sink.
///
/// The bounded loop owns one `MAX_CHUNK_BYTES` read buffer and hands the sink one borrowed
/// prefix of that same buffer, so nothing scales with the volume a child produces.
pub const RESIDENT_OUTPUT_BYTES: usize = crate::output::MAX_CHUNK_BYTES;

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
    let mut child = match spawn(&invocation) {
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
    let streamed = pipes::stream(
        child.stdout.take(),
        child.stderr.take(),
        &mut budget,
        sink,
        deadline,
        process_group,
    );
    let ending = wait_for_child(&mut child, streamed.kill_by, process_group);
    descendants::kill_group(process_group);
    descendants::reap_group(process_group);
    pid1::reap_orphans();
    descendants::sweep_strays();
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

fn wait_for_child(child: &mut Child, until: Instant, process_group: i32) -> Ending {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return ending(status),
            Ok(None) if Instant::now() < until => thread::sleep(WAIT_POLL),
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
