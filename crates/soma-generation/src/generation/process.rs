use std::{
    ffi::OsString,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use sha2::{Digest as _, Sha256};

use super::{
    artifacts::Sha256Digest,
    error::{CompileError, CompileErrorKind, CompilePhase},
};

const CAPTURE_LIMIT: usize = 64 * 1024;
const MAX_TOOL_BYTES: u64 = 256 * 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(5);

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
    pub(crate) fn run_with_stdin(
        self,
        feed: impl FnOnce(&mut dyn Write) -> Result<(), CompileError>,
    ) -> Result<ToolOutcome, CompileError> {
        let phase = self.phase;
        let mut command = Command::new(self.program);
        command
            .args(&self.arguments)
            .env_clear()
            .envs(self.environment.iter().map(|(key, value)| (key, value)))
            .current_dir(self.working_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|_| CompileError::new(phase, CompileErrorKind::Toolchain))?;
        let mut stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (feed_result, stdout, stderr) = thread::scope(|scope| {
            let out = scope.spawn(move || drain(stdout));
            let err = scope.spawn(move || drain(stderr));
            let feed_result = match stdin.as_mut() {
                Some(pipe) => feed(pipe),
                None => Ok(()),
            };
            drop(stdin);
            if feed_result.is_err() {
                let _ = child.kill();
            }
            let outcome = wait_bounded(&mut child, self.deadline);
            (
                feed_result.and(outcome),
                out.join().unwrap_or_default(),
                err.join().unwrap_or_default(),
            )
        });
        let exit_code = feed_result?;
        Ok(ToolOutcome {
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
        })
    }
}

fn wait_bounded(child: &mut Child, deadline: Duration) -> Result<Option<i32>, CompileError> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.code()),
            Ok(None) if started.elapsed() < deadline => thread::sleep(POLL_INTERVAL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CompileError::new(
                    CompilePhase::FormatRoot,
                    CompileErrorKind::Toolchain,
                ));
            }
            Err(_) => {
                return Err(CompileError::new(
                    CompilePhase::FormatRoot,
                    CompileErrorKind::Io,
                ));
            }
        }
    }
}

fn drain(source: Option<impl Read>) -> Vec<u8> {
    let Some(mut source) = source else {
        return Vec::new();
    };
    let mut retained = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        match source.read(&mut buffer) {
            Ok(0) | Err(_) => return retained,
            Ok(count) => {
                let room = CAPTURE_LIMIT.saturating_sub(retained.len());
                retained.extend_from_slice(&buffer[..count.min(room)]);
            }
        }
    }
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
