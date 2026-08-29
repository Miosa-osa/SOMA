use std::fmt;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Serialize, Serializer, ser::SerializeStruct as _};

use crate::{InstanceId, PublishedPort, process::ProcessOutput, request::MAX_OUTPUT_BYTES};

use super::MachineResources;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutionStatus {
    Exited { code: i32 },
    Signaled,
    TimedOut,
    OutputLimitExceeded,
}

impl ExecutionStatus {
    #[must_use]
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Exited { code: 0 })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupState {
    Complete,
}

#[derive(Clone, Eq, PartialEq)]
pub struct ExecutionResult {
    instance_id: InstanceId,
    status: ExecutionStatus,
    stdout: Box<[u8]>,
    stdout_observed_bytes: u64,
    stderr: Box<[u8]>,
    stderr_observed_bytes: u64,
    elapsed_millis: u64,
    cleanup: Option<CleanupState>,
    resources: Option<MachineResources>,
    cleanup_published_ports: Option<Vec<PublishedPort>>,
}

impl ExecutionResult {
    pub(crate) fn from_process(
        instance_id: InstanceId,
        output: ProcessOutput,
        cleanup: Option<CleanupState>,
        resources: Option<MachineResources>,
        cleanup_published_ports: Option<Vec<PublishedPort>>,
    ) -> Self {
        let (status, stdout, stdout_observed_bytes, stderr, stderr_observed_bytes, elapsed_millis) =
            output.into_observed_parts();
        Self {
            instance_id,
            status,
            stdout: stdout.into_boxed_slice(),
            stdout_observed_bytes,
            stderr: stderr.into_boxed_slice(),
            stderr_observed_bytes,
            elapsed_millis,
            cleanup,
            resources,
            cleanup_published_ports,
        }
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub const fn status(&self) -> ExecutionStatus {
        self.status
    }

    #[must_use]
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    #[must_use]
    pub const fn stdout_observed_bytes(&self) -> u64 {
        self.stdout_observed_bytes
    }

    #[must_use]
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    #[must_use]
    pub const fn stderr_observed_bytes(&self) -> u64 {
        self.stderr_observed_bytes
    }

    #[must_use]
    pub const fn elapsed_millis(&self) -> u64 {
        self.elapsed_millis
    }

    #[must_use]
    pub const fn cleanup(&self) -> Option<CleanupState> {
        self.cleanup
    }

    #[must_use]
    pub const fn resources(&self) -> Option<MachineResources> {
        self.resources
    }

    #[must_use]
    pub fn cleanup_published_ports(&self) -> Option<&[PublishedPort]> {
        self.cleanup_published_ports.as_deref()
    }
}

impl Serialize for ExecutionResult {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let combined_bytes = self
            .stdout
            .len()
            .checked_add(self.stderr.len())
            .ok_or_else(|| serde::ser::Error::custom("guest output length overflow"))?;
        if combined_bytes > usize::try_from(MAX_OUTPUT_BYTES).unwrap_or(usize::MAX) {
            return Err(serde::ser::Error::custom(
                "guest output exceeds the declared 16 MiB allowance",
            ));
        }

        let mut result = serializer.serialize_struct("ExecutionResult", 9)?;
        result.serialize_field("instance_id", &self.instance_id)?;
        result.serialize_field("status", &self.status)?;
        result.serialize_field("stdout", &EncodedBytes(&self.stdout))?;
        result.serialize_field("stdout_observed_bytes", &self.stdout_observed_bytes)?;
        result.serialize_field("stderr", &EncodedBytes(&self.stderr))?;
        result.serialize_field("stderr_observed_bytes", &self.stderr_observed_bytes)?;
        result.serialize_field("elapsed_millis", &self.elapsed_millis)?;
        result.serialize_field("cleanup", &self.cleanup)?;
        result.serialize_field("resources", &self.resources)?;
        result.end()
    }
}

impl fmt::Debug for ExecutionResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutionResult")
            .field("instance_id", &self.instance_id)
            .field("status", &self.status)
            .field("stdout_bytes", &self.stdout.len())
            .field("stdout_observed_bytes", &self.stdout_observed_bytes)
            .field("stderr_bytes", &self.stderr.len())
            .field("stderr_observed_bytes", &self.stderr_observed_bytes)
            .field("elapsed_millis", &self.elapsed_millis)
            .field("cleanup", &self.cleanup)
            .field("resources", &self.resources)
            .field(
                "cleanup_published_ports",
                &self.cleanup_published_ports.as_ref().map(Vec::len),
            )
            .finish()
    }
}

struct EncodedBytes<'a>(&'a [u8]);

impl Serialize for EncodedBytes<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let encoded = STANDARD.encode(self.0);
        let mut output = serializer.serialize_struct("EncodedBytes", 3)?;
        output.serialize_field("encoding", "base64")?;
        output.serialize_field("byte_length", &self.0.len())?;
        output.serialize_field("data", &encoded)?;
        output.end()
    }
}
