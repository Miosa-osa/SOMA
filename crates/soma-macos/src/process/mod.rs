mod system;

use std::{ffi::OsString, path::PathBuf, time::Duration};

pub(crate) use system::SystemProcessRunner;

use crate::{ExecutionStatus, ProcessFailureKind};

#[derive(Clone)]
pub(crate) struct ProcessInvocation {
    program: PathBuf,
    arguments: Vec<OsString>,
    timeout: Duration,
    output_limit: usize,
}

impl ProcessInvocation {
    pub(crate) fn new(
        program: PathBuf,
        arguments: Vec<OsString>,
        timeout: Duration,
        output_limit: usize,
    ) -> Self {
        Self {
            program,
            arguments,
            timeout,
            output_limit,
        }
    }

    pub(crate) fn program(&self) -> &PathBuf {
        &self.program
    }

    pub(crate) fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    pub(crate) const fn timeout(&self) -> Duration {
        self.timeout
    }

    pub(crate) const fn output_limit(&self) -> usize {
        self.output_limit
    }
}

pub(crate) trait ProcessRunner: Send + Sync {
    fn run(&self, invocation: &ProcessInvocation) -> Result<ProcessOutput, ProcessFailureKind>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProcessOutput {
    status: ExecutionStatus,
    stdout: Vec<u8>,
    stdout_observed_bytes: u64,
    stderr: Vec<u8>,
    stderr_observed_bytes: u64,
    elapsed_millis: u64,
}

impl ProcessOutput {
    #[cfg(test)]
    pub(crate) fn new(
        status: ExecutionStatus,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        elapsed_millis: u64,
    ) -> Self {
        let stdout_observed_bytes = u64::try_from(stdout.len()).unwrap_or(u64::MAX);
        let stderr_observed_bytes = u64::try_from(stderr.len()).unwrap_or(u64::MAX);
        Self {
            status,
            stdout,
            stdout_observed_bytes,
            stderr,
            stderr_observed_bytes,
            elapsed_millis,
        }
    }

    pub(crate) const fn with_observed_bytes(
        status: ExecutionStatus,
        stdout: Vec<u8>,
        stdout_observed_bytes: u64,
        stderr: Vec<u8>,
        stderr_observed_bytes: u64,
        elapsed_millis: u64,
    ) -> Self {
        Self {
            status,
            stdout,
            stdout_observed_bytes,
            stderr,
            stderr_observed_bytes,
            elapsed_millis,
        }
    }

    pub(crate) const fn status(&self) -> ExecutionStatus {
        self.status
    }

    pub(crate) fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    pub(crate) const fn elapsed_millis(&self) -> u64 {
        self.elapsed_millis
    }

    pub(crate) fn into_observed_parts(self) -> (ExecutionStatus, Vec<u8>, u64, Vec<u8>, u64, u64) {
        (
            self.status,
            self.stdout,
            self.stdout_observed_bytes,
            self.stderr,
            self.stderr_observed_bytes,
            self.elapsed_millis,
        )
    }
}

pub(crate) fn constrain_output(mut output: ProcessOutput, limit: usize) -> ProcessOutput {
    if output.stdout.len().saturating_add(output.stderr.len()) <= limit {
        return output;
    }
    output.stdout.truncate(limit);
    output
        .stderr
        .truncate(limit.saturating_sub(output.stdout.len()));
    output.status = ExecutionStatus::OutputLimitExceeded;
    output
}
