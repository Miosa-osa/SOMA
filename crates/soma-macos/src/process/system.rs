use std::{
    io::{self, Read},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crate::{ExecutionStatus, ProcessFailureKind};

use super::{ProcessInvocation, ProcessOutput, ProcessRunner};

const POLL_INTERVAL: Duration = Duration::from_millis(2);

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemProcessRunner;

impl ProcessRunner for SystemProcessRunner {
    fn run(&self, invocation: &ProcessInvocation) -> Result<ProcessOutput, ProcessFailureKind> {
        let started = Instant::now();
        let mut child = Command::new(invocation.program())
            .args(invocation.arguments())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| map_spawn_error(&error))?;

        let stdout = take_stdout(&mut child)?;
        let stderr = take_stderr(&mut child)?;
        let budget = Arc::new(CaptureBudget::new(invocation.output_limit()));
        let stdout_reader = capture(stdout, Arc::clone(&budget));
        let stderr_reader = capture(stderr, Arc::clone(&budget));

        let observed_status = loop {
            if budget.failed.load(Ordering::Acquire) {
                terminate(&mut child)?;
                break ExecutionStatus::Signaled;
            }
            if budget.exceeded.load(Ordering::Acquire) {
                terminate(&mut child)?;
                break ExecutionStatus::OutputLimitExceeded;
            }
            if started.elapsed() >= invocation.timeout() {
                terminate(&mut child)?;
                break ExecutionStatus::TimedOut;
            }
            match child
                .try_wait()
                .map_err(|_| ProcessFailureKind::WaitFailed)?
            {
                Some(exit) => {
                    break exit.code().map_or(ExecutionStatus::Signaled, |code| {
                        ExecutionStatus::Exited { code }
                    });
                }
                None => thread::sleep(POLL_INTERVAL),
            }
        };

        let stdout = stdout_reader
            .join()
            .map_err(|_| ProcessFailureKind::ReaderPanicked)??;
        let stderr = stderr_reader
            .join()
            .map_err(|_| ProcessFailureKind::ReaderPanicked)??;
        if budget.failed.load(Ordering::Acquire) {
            return Err(ProcessFailureKind::ReadFailed);
        }
        let status = if budget.exceeded.load(Ordering::Acquire) {
            ExecutionStatus::OutputLimitExceeded
        } else {
            observed_status
        };

        Ok(ProcessOutput::with_observed_bytes(
            status,
            stdout.bytes,
            stdout.observed_bytes,
            stderr.bytes,
            stderr.observed_bytes,
            elapsed_millis(started.elapsed()),
        ))
    }
}

struct CaptureBudget {
    remaining: AtomicUsize,
    exceeded: AtomicBool,
    failed: AtomicBool,
}

impl CaptureBudget {
    const fn new(limit: usize) -> Self {
        Self {
            remaining: AtomicUsize::new(limit),
            exceeded: AtomicBool::new(false),
            failed: AtomicBool::new(false),
        }
    }

    fn claim(&self, requested: usize) -> usize {
        let mut current = self.remaining.load(Ordering::Acquire);
        loop {
            let granted = current.min(requested);
            match self.remaining.compare_exchange_weak(
                current,
                current - granted,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return granted,
                Err(observed) => current = observed,
            }
        }
    }
}

fn capture(
    mut stream: impl Read + Send + 'static,
    budget: Arc<CaptureBudget>,
) -> thread::JoinHandle<Result<CapturedStream, ProcessFailureKind>> {
    thread::spawn(move || {
        let mut captured = Vec::new();
        let mut observed_bytes = 0_u64;
        let mut chunk = [0_u8; 8_192];
        loop {
            let Ok(read) = stream.read(&mut chunk) else {
                budget.failed.store(true, Ordering::Release);
                return Err(ProcessFailureKind::ReadFailed);
            };
            if read == 0 {
                return Ok(CapturedStream {
                    bytes: captured,
                    observed_bytes,
                });
            }
            observed_bytes = observed_bytes.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
            let granted = budget.claim(read);
            captured.extend_from_slice(&chunk[..granted]);
            if granted < read {
                budget.exceeded.store(true, Ordering::Release);
                return Ok(CapturedStream {
                    bytes: captured,
                    observed_bytes,
                });
            }
        }
    })
}

struct CapturedStream {
    bytes: Vec<u8>,
    observed_bytes: u64,
}

fn terminate(child: &mut std::process::Child) -> Result<(), ProcessFailureKind> {
    if child
        .try_wait()
        .map_err(|_| ProcessFailureKind::WaitFailed)?
        .is_some()
    {
        return Ok(());
    }
    child.kill().map_err(|_| ProcessFailureKind::KillFailed)?;
    child.wait().map_err(|_| ProcessFailureKind::WaitFailed)?;
    Ok(())
}

fn take_stdout(
    child: &mut std::process::Child,
) -> Result<std::process::ChildStdout, ProcessFailureKind> {
    if let Some(stdout) = child.stdout.take() {
        Ok(stdout)
    } else {
        terminate(child)?;
        Err(ProcessFailureKind::PipeUnavailable)
    }
}

fn take_stderr(
    child: &mut std::process::Child,
) -> Result<std::process::ChildStderr, ProcessFailureKind> {
    if let Some(stderr) = child.stderr.take() {
        Ok(stderr)
    } else {
        terminate(child)?;
        Err(ProcessFailureKind::PipeUnavailable)
    }
}

fn map_spawn_error(error: &io::Error) -> ProcessFailureKind {
    match error.kind() {
        io::ErrorKind::NotFound => ProcessFailureKind::ExecutableUnavailable,
        io::ErrorKind::PermissionDenied => ProcessFailureKind::PermissionDenied,
        _ => ProcessFailureKind::SpawnFailed,
    }
}

fn elapsed_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
