//! One bounded, contained invocation of one pinned external tool.
//!
//! Every tool is a [`PinnedTool`]: opened once, hashed through that descriptor, and executed
//! through that same descriptor, so provenance names the process image that ran.
//!
//! Containment itself belongs to [`soma_supervise`]: the tool leads its own process group, so
//! a deadline, a feed failure, a capture overflow, or a cancellation terminates the tool and
//! every descendant it forked rather than only the direct child, and termination, draining,
//! waiting, and collection are each bounded.
//! This module adds only what the compiler owns: the pinned descriptor, the explicit
//! environment, and the phase that names the failure.

use std::{
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use soma_supervise::{Contained, Uncontained};

use super::error::{CompileError, CompileErrorKind, CompilePhase};

mod control;
mod pinned;

pub(crate) use pinned::PinnedTool;
#[cfg(all(test, target_os = "linux"))]
pub(crate) use soma_supervise::TERMINATION_GRACE;

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
    pub(crate) program: &'a PinnedTool,
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
        let command = self.compose()?;
        let output = Contained::new(command, self.deadline)
            .run(feed)
            .map_err(|failure| match failure {
                Uncontained::Input(error) => error,
                Uncontained::Lost => CompileError::new(phase, CompileErrorKind::Io),
                Uncontained::Spawn | Uncontained::Terminated => {
                    CompileError::new(phase, CompileErrorKind::Toolchain)
                }
            })?;
        Ok(self.outcome(output.exit_code, output.stdout, output.stderr))
    }

    fn compose(&self) -> Result<Command, CompileError> {
        self.program.require_bound(self.phase)?;
        let mut command = Command::new(self.program.program());
        control::inherit_tool(&mut command, self.program.descriptor());
        command
            .args(&self.arguments)
            .env_clear()
            .envs(self.environment.iter().map(|(key, value)| (key, value)))
            .current_dir(self.working_directory);
        Ok(command)
    }

    fn outcome(self, exit_code: Option<i32>, stdout: Vec<u8>, stderr: Vec<u8>) -> ToolOutcome {
        ToolOutcome {
            program: self.program.name().to_owned(),
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

/// Runs `program -V` (or the given flag) and returns the first bounded output line.
pub(crate) fn version_line(
    program: &PinnedTool,
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

#[cfg(all(test, target_os = "linux"))]
mod tests;
