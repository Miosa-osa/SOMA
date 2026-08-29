use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq)]
pub struct ObservedOutput {
    stdout: Vec<u8>,
    stdout_observed_bytes: u64,
    stderr: Vec<u8>,
    stderr_observed_bytes: u64,
}

impl ObservedOutput {
    #[must_use]
    pub const fn new(
        stdout: Vec<u8>,
        stdout_observed_bytes: u64,
        stderr: Vec<u8>,
        stderr_observed_bytes: u64,
    ) -> Self {
        Self {
            stdout,
            stdout_observed_bytes,
            stderr,
            stderr_observed_bytes,
        }
    }

    pub(crate) fn into_parts(self) -> (Vec<u8>, u64, Vec<u8>, u64) {
        (
            self.stdout,
            self.stdout_observed_bytes,
            self.stderr,
            self.stderr_observed_bytes,
        )
    }
}

impl fmt::Debug for ObservedOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObservedOutput")
            .field("stdout_bytes", &self.stdout.len())
            .field("stdout_observed_bytes", &self.stdout_observed_bytes)
            .field("stderr_bytes", &self.stderr.len())
            .field("stderr_observed_bytes", &self.stderr_observed_bytes)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct CapturedOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl CapturedOutput {
    pub(crate) const fn new(stdout: Vec<u8>, stderr: Vec<u8>) -> Self {
        Self { stdout, stderr }
    }

    #[must_use]
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    #[must_use]
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }
}

impl fmt::Debug for CapturedOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapturedOutput")
            .field("stdout_bytes", &self.stdout.len())
            .field("stderr_bytes", &self.stderr.len())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamMetadata {
    captured_bytes: u64,
    observed_bytes: u64,
    truncated: bool,
    sha256: String,
}

impl StreamMetadata {
    pub(crate) const fn new(
        captured_bytes: u64,
        observed_bytes: u64,
        truncated: bool,
        sha256: String,
    ) -> Self {
        Self {
            captured_bytes,
            observed_bytes,
            truncated,
            sha256,
        }
    }

    #[must_use]
    pub const fn captured_bytes(&self) -> u64 {
        self.captured_bytes
    }

    #[must_use]
    pub const fn observed_bytes(&self) -> u64 {
        self.observed_bytes
    }

    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    fn is_valid(&self) -> bool {
        self.captured_bytes <= self.observed_bytes
            && self.truncated == (self.captured_bytes < self.observed_bytes)
            && valid_sha256(&self.sha256)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputMetadata {
    stdout: StreamMetadata,
    stderr: StreamMetadata,
}

impl OutputMetadata {
    pub(crate) const fn new(stdout: StreamMetadata, stderr: StreamMetadata) -> Self {
        Self { stdout, stderr }
    }

    #[must_use]
    pub const fn stdout(&self) -> &StreamMetadata {
        &self.stdout
    }

    #[must_use]
    pub const fn stderr(&self) -> &StreamMetadata {
        &self.stderr
    }

    pub(crate) fn is_valid(&self) -> bool {
        self.stdout.is_valid() && self.stderr.is_valid()
    }
}

fn valid_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}
