//! Bounded direct command execution: argv `execve`, fixed environment, own process group,
//! piped output with exact accounting, absolute deadline, and complete descendant reaping.

use std::io::Read;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use soma_guest::{GuestCommand, TerminalStatus};

use crate::descendants;
use crate::environment::{ENVIRONMENT, InvalidInvocation, Invocation, WORKING_DIRECTORY};
use crate::output::{Admission, Ending, MAX_CHUNK_BYTES, OutputBudget, terminal_status};
use crate::pid1;

const KILL_GRACE: Duration = Duration::from_millis(500);
const WAIT_POLL: Duration = Duration::from_millis(5);

/// Destination for admitted output chunks.
pub trait OutputSink {
    /// Delivers one nonempty admitted stdout chunk.
    ///
    /// # Errors
    ///
    /// Returns [`SinkFault`] if the chunk could not be delivered; execution is then aborted.
    fn stdout(&mut self, bytes: Vec<u8>) -> Result<(), SinkFault>;
    /// Delivers one nonempty admitted stderr chunk.
    ///
    /// # Errors
    ///
    /// Returns [`SinkFault`] if the chunk could not be delivered; execution is then aborted.
    fn stderr(&mut self, bytes: Vec<u8>) -> Result<(), SinkFault>;
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
}

#[derive(Clone, Copy)]
enum Stream {
    Stdout,
    Stderr,
}

enum Event {
    Data(Stream, Vec<u8>),
    Closed,
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
            });
        }
    };
    let process_group = i32::try_from(child.id()).unwrap_or(0);
    if process_group <= 1 {
        let _ = child.kill();
        let _ = child.wait();
        return Err(ExecutorFault::ProcessGroup);
    }
    let receiver = pump(&mut child);
    let mut budget = OutputBudget::new(command.output_bytes());
    let streamed = stream_output(&receiver, &mut budget, sink, deadline, process_group);
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

fn pump(child: &mut Child) -> Receiver<Event> {
    let (sender, receiver) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        spawn_reader(stdout, Stream::Stdout, sender.clone());
    } else {
        let _ = sender.send(Event::Closed);
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_reader(stderr, Stream::Stderr, sender);
    } else {
        let _ = sender.send(Event::Closed);
    }
    receiver
}

fn spawn_reader(mut source: impl Read + Send + 'static, stream: Stream, sender: Sender<Event>) {
    thread::spawn(move || {
        let mut buffer = [0; MAX_CHUNK_BYTES];
        loop {
            match source.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if sender
                        .send(Event::Data(stream, buffer[..count].to_vec()))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
        let _ = sender.send(Event::Closed);
    });
}

struct Streamed {
    limit_hit: bool,
    timed_out: bool,
    sink_failed: bool,
    kill_by: Instant,
}

fn stream_output(
    receiver: &Receiver<Event>,
    budget: &mut OutputBudget,
    sink: &mut impl OutputSink,
    deadline: Instant,
    process_group: i32,
) -> Streamed {
    let mut streamed = Streamed {
        limit_hit: false,
        timed_out: false,
        sink_failed: false,
        kill_by: deadline,
    };
    let mut open = 2;
    let mut killed = false;
    let kill = |streamed: &mut Streamed| {
        descendants::kill_group(process_group);
        streamed.kill_by = Instant::now() + KILL_GRACE;
    };
    while open > 0 {
        let remaining = streamed.kill_by.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(remaining) {
            Ok(Event::Closed) => open -= 1,
            Ok(Event::Data(..)) if killed => {}
            Ok(Event::Data(stream, bytes)) => {
                let (chunk, reached_limit) = match budget.admit(bytes) {
                    Admission::Admitted(chunk) => (chunk, false),
                    Admission::Limit(chunk) => (chunk, true),
                    Admission::Exhausted => (Vec::new(), true),
                };
                if !chunk.is_empty() && deliver(sink, stream, chunk).is_err() {
                    streamed.sink_failed = true;
                    killed = true;
                    kill(&mut streamed);
                }
                if reached_limit {
                    streamed.limit_hit = true;
                    killed = true;
                    kill(&mut streamed);
                }
            }
            Err(RecvTimeoutError::Timeout) if killed => break,
            Err(RecvTimeoutError::Timeout) => {
                streamed.timed_out = true;
                killed = true;
                kill(&mut streamed);
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    streamed
}

fn deliver(sink: &mut impl OutputSink, stream: Stream, chunk: Vec<u8>) -> Result<(), SinkFault> {
    match stream {
        Stream::Stdout => sink.stdout(chunk),
        Stream::Stderr => sink.stderr(chunk),
    }
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
