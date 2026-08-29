//! One bounded, contained invocation of one pinned external tool.
//!
//! Every tool leads its own process group, so a deadline, a feed failure, a capture failure, or
//! a cancellation terminates the tool and every descendant it forked rather than only the
//! direct child.
//! Termination, draining, waiting, and collection are each bounded, and every error carries the
//! phase that actually invoked the tool.

use std::{
    ffi::OsString,
    fs::File,
    io::{Read as _, Write},
    path::{Path, PathBuf},
    process::{ChildStdin, Command, Stdio},
    time::{Duration, Instant},
};

use sha2::{Digest as _, Sha256};

use super::{
    artifacts::Sha256Digest,
    error::{CompileError, CompileErrorKind, CompilePhase},
};

mod capture;
mod control;
mod supervise;

use capture::Readers;
use control::Group;
use supervise::Supervisor;

const MAX_TOOL_BYTES: u64 = 256 * 1024 * 1024;
/// Grace the process group is given to honor the polite termination signal.
const TERM_GRACE: Duration = Duration::from_secs(2);
/// Grace the process group is given to die after the force signal.
const KILL_GRACE: Duration = Duration::from_secs(2);
/// Grace the detached readers are given to report before they are abandoned.
const CAPTURE_GRACE: Duration = Duration::from_secs(2);

/// The complete bounded overrun one invocation may add to its own deadline.
///
/// A tool that ignores its deadline costs at most this long in polite termination, forced
/// termination, and output collection before the phase reports its failure.
pub(crate) const TERMINATION_GRACE: Duration = TERM_GRACE
    .saturating_add(KILL_GRACE)
    .saturating_add(CAPTURE_GRACE)
    .saturating_add(CAPTURE_GRACE);

/// Retained evidence from one bounded pinned-tool invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolOutcome {
    /// The program name without its host directory.
    pub program: String,
    /// The exact arguments passed without any shell interpretation.
    pub arguments: Vec<String>,
    /// The explicit environment given to the tool; nothing else was inherited.
    pub environment: Vec<(String, String)>,
    /// The process exit code, or `None` when the process was killed or signalled.
    pub exit_code: Option<i32>,
    /// The first 64 KiB of standard output.
    pub stdout: Vec<u8>,
    /// The first 64 KiB of standard error.
    pub stderr: Vec<u8>,
}

impl ToolOutcome {
    pub(crate) fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }
}

/// One typed tool invocation with no shell, inherited environment, or working-directory guess.
pub(crate) struct Invocation<'a> {
    pub(crate) program: &'a Path,
    pub(crate) arguments: Vec<OsString>,
    pub(crate) environment: Vec<(String, String)>,
    pub(crate) working_directory: &'a Path,
    pub(crate) deadline: Duration,
    pub(crate) phase: CompilePhase,
}

impl Invocation<'_> {
    pub(crate) fn run(self) -> Result<ToolOutcome, CompileError> {
        self.run_with_stdin(|_| Ok(()))
    }

    /// Runs the tool while `feed` writes its standard input from the calling thread.
    ///
    /// The feed is bounded by the same supervisor: when the deadline passes, the group is
    /// terminated, the tool's read end closes, and a blocked write fails instead of hanging.
    pub(crate) fn run_with_stdin(
        self,
        feed: impl FnOnce(&mut dyn Write) -> Result<(), CompileError>,
    ) -> Result<ToolOutcome, CompileError> {
        let phase = self.phase;
        let mut child = self.spawn()?;
        let group = Group::new(child.id());
        let deadline = Instant::now() + self.deadline;
        let mut readers = Readers::spawn(child.stdout.take(), child.stderr.take());
        let stdin = child.stdin.take();
        let supervisor = Supervisor::start(child, group, deadline, (TERM_GRACE, KILL_GRACE));
        let feed_result = feed_stdin(stdin, feed);
        if feed_result.is_err() {
            supervisor.cancel();
        }
        let collected = supervisor.finish(deadline + TERMINATION_GRACE);
        // An incomplete collection proves a descendant still holds a build pipe, which also
        // proves the group still has a member and its identifier is still reserved, so forcing
        // the group here can never reach an unrelated process.
        let contained = readers.collect(Instant::now() + CAPTURE_GRACE);
        if !contained {
            group.signal(control::Signal::Force);
            readers.collect(Instant::now() + CAPTURE_GRACE);
        }
        let captured = readers.take();
        feed_result?;
        let Some(supervised) = collected else {
            return Err(CompileError::new(phase, CompileErrorKind::Io));
        };
        if supervised.terminated || !contained {
            return Err(CompileError::new(phase, CompileErrorKind::Toolchain));
        }
        Ok(self.outcome(supervised.exit_code, captured.stdout, captured.stderr))
    }

    fn spawn(&self) -> Result<std::process::Child, CompileError> {
        let mut command = Command::new(self.program);
        command
            .args(&self.arguments)
            .env_clear()
            .envs(self.environment.iter().map(|(key, value)| (key, value)))
            .current_dir(self.working_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        control::isolate(&mut command);
        command
            .spawn()
            .map_err(|_| CompileError::new(self.phase, CompileErrorKind::Toolchain))
    }

    fn outcome(self, exit_code: Option<i32>, stdout: Vec<u8>, stderr: Vec<u8>) -> ToolOutcome {
        ToolOutcome {
            program: self
                .program
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            arguments: self
                .arguments
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect(),
            environment: self.environment,
            exit_code,
            stdout,
            stderr,
        }
    }
}

fn feed_stdin(
    stdin: Option<ChildStdin>,
    feed: impl FnOnce(&mut dyn Write) -> Result<(), CompileError>,
) -> Result<(), CompileError> {
    let Some(mut pipe) = stdin else {
        return feed(&mut std::io::sink());
    };
    let result = feed(&mut pipe);
    drop(pipe);
    result
}

/// Hashes the bytes of one pinned tool executable so evidence binds the exact binary used.
pub(crate) fn executable_digest(
    program: &Path,
    phase: CompilePhase,
) -> Result<Sha256Digest, CompileError> {
    let mut file =
        File::open(program).map_err(|_| CompileError::new(phase, CompileErrorKind::Toolchain))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| CompileError::new(phase, CompileErrorKind::Io))?;
        if count == 0 {
            break;
        }
        total +=
            u64::try_from(count).map_err(|_| CompileError::new(phase, CompileErrorKind::Io))?;
        if total > MAX_TOOL_BYTES {
            return Err(CompileError::new(phase, CompileErrorKind::LimitExceeded));
        }
        hasher.update(&buffer[..count]);
    }
    let mut digest = [0_u8; 32];
    digest.copy_from_slice(hasher.finalize().as_ref());
    Ok(Sha256Digest::from_bytes(digest))
}

/// Runs `program -V` (or the given flag) and returns the first bounded output line.
pub(crate) fn version_line(
    program: &Path,
    flag: &str,
    working_directory: &Path,
    phase: CompilePhase,
) -> Result<String, CompileError> {
    let outcome = Invocation {
        program,
        arguments: vec![OsString::from(flag)],
        environment: Vec::new(),
        working_directory,
        deadline: Duration::from_secs(10),
        phase,
    }
    .run()?;
    if !outcome.succeeded() {
        return Err(CompileError::new(phase, CompileErrorKind::Toolchain));
    }
    let combined = if outcome.stdout.is_empty() {
        outcome.stderr
    } else {
        outcome.stdout
    };
    let line = combined.split(|byte| *byte == b'\n').next().unwrap_or(&[]);
    Ok(String::from_utf8_lossy(line).into_owned())
}

/// Locates one tool by exact file name inside an explicit directory.
pub(crate) fn tool_path(directory: &Path, name: &str) -> PathBuf {
    directory.join(name)
}

#[cfg(test)]
mod tests;
