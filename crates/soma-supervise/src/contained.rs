//! The bounded orchestration one caller sees: spawn, feed, supervise, collect, judge.

use std::{
    io::Write,
    process::{ChildStdin, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

use super::{
    capture::Readers,
    group::{self, Group, Signal},
    supervise::Supervisor,
};

/// Grace the process group is given to honor the polite termination signal.
const TERM_GRACE: Duration = Duration::from_secs(2);
/// Grace the process group is given to die after the force signal.
const KILL_GRACE: Duration = Duration::from_secs(2);
/// Grace the detached readers are given to report before they are abandoned.
const CAPTURE_GRACE: Duration = Duration::from_secs(2);

/// The complete bounded overrun one invocation may add to its own deadline.
///
/// A tool that ignores its deadline costs at most this long in polite termination, forced
/// termination, and output collection before the caller reports its failure.
pub const TERMINATION_GRACE: Duration = TERM_GRACE
    .saturating_add(KILL_GRACE)
    .saturating_add(CAPTURE_GRACE)
    .saturating_add(CAPTURE_GRACE);

/// What one contained invocation produced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Output {
    /// The process exit code, or `None` when the process was killed or signalled.
    pub exit_code: Option<i32>,
    /// The retained standard output, at most [`super::CAPTURE_LIMIT`] bytes.
    pub stdout: Vec<u8>,
    /// The retained standard error, at most [`super::CAPTURE_LIMIT`] bytes.
    pub stderr: Vec<u8>,
}

impl Output {
    /// Whether the tool exited with status zero.
    #[must_use]
    pub const fn succeeded(&self) -> bool {
        matches!(self.exit_code, Some(0))
    }
}

/// Why one invocation produced no trustworthy result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Uncontained<E> {
    /// The program could not be started at all.
    Spawn,
    /// The caller's standard-input feed failed; the group was terminated first.
    Input(E),
    /// The deadline, a capture overflow, or a descendant holding the pipes forced termination.
    Terminated,
    /// The supervisor did not report within its own bound, so the outcome is unknown.
    Lost,
}

/// One typed tool invocation with no shell and no implicit timeout.
pub struct Contained {
    command: Command,
    deadline: Duration,
}

impl Contained {
    /// Takes one fully configured command; this module owns its standard streams.
    #[must_use]
    pub const fn new(command: Command, deadline: Duration) -> Self {
        Self { command, deadline }
    }

    /// Runs the tool while `feed` writes its standard input from the calling thread.
    ///
    /// The feed is bounded by the same supervisor: when the deadline passes, the group is
    /// terminated, the tool's read end closes, and a blocked write fails instead of hanging.
    /// A tool that is spawned always ends: it exits, or the group is signalled and reaped.
    ///
    /// # Errors
    ///
    /// Returns [`Uncontained`] naming the bound that was reached; a feed failure is reported
    /// as the caller's own error after the group has been terminated.
    pub fn run<E>(
        mut self,
        feed: impl FnOnce(&mut dyn Write) -> Result<(), E>,
    ) -> Result<Output, Uncontained<E>> {
        self.command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        group::isolate(&mut self.command);
        let mut child = self.command.spawn().map_err(|_| Uncontained::Spawn)?;
        let group = Group::new(child.id());
        let deadline = Instant::now() + self.deadline;
        let (cancel, requests) = mpsc::channel();
        let mut readers = Readers::spawn(child.stdout.take(), child.stderr.take(), &cancel);
        let stdin = child.stdin.take();
        let supervisor =
            Supervisor::start(child, group, deadline, (TERM_GRACE, KILL_GRACE), requests);
        let feed_result = feed_stdin(stdin, feed);
        if feed_result.is_err() {
            let _ = cancel.send(());
        }
        let collected = supervisor.finish(deadline + TERMINATION_GRACE);
        // An incomplete collection proves a descendant still holds a pipe, which also proves
        // the group still has a member and its identifier is still reserved, so forcing the
        // group here can never reach an unrelated process.
        let complete = readers.collect(Instant::now() + CAPTURE_GRACE);
        if !complete {
            group.signal(Signal::Force);
            readers.collect(Instant::now() + CAPTURE_GRACE);
        }
        let captured = readers.take();
        drop(cancel);
        feed_result.map_err(Uncontained::Input)?;
        let supervised = collected.ok_or(Uncontained::Lost)?;
        if supervised.terminated || !complete || captured.overflowed {
            return Err(Uncontained::Terminated);
        }
        Ok(Output {
            exit_code: supervised.exit_code,
            stdout: captured.stdout,
            stderr: captured.stderr,
        })
    }
}

fn feed_stdin<E>(
    stdin: Option<ChildStdin>,
    feed: impl FnOnce(&mut dyn Write) -> Result<(), E>,
) -> Result<(), E> {
    let Some(mut pipe) = stdin else {
        return feed(&mut std::io::sink());
    };
    let result = feed(&mut pipe);
    drop(pipe);
    result
}

#[cfg(test)]
mod tests;
