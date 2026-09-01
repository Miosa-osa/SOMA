use std::fmt;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Serialize, Serializer, ser::SerializeStruct as _};
use soma::{BackendKind, CommandStatus, ExecutionReceipt, InstanceId, MachineState};

mod file;
mod list;
mod pty;

pub use file::FileReport;
pub use list::SandboxListReport;
pub use pty::PtyReport;

pub const ENVELOPE_SCHEMA: &str = "soma.cli.v1";
pub const MAX_OUTPUT_BYTES_USIZE: usize = 16 * 1024 * 1024;

#[allow(
    clippy::cast_possible_truncation,
    reason = "this compile-time guard fails if the portable facade limit stops fitting usize"
)]
const FACADE_MAX_OUTPUT_BYTES_USIZE: usize = soma::ExecutionLimits::MAX_OUTPUT_BYTES as usize;
const _: () = assert!(MAX_OUTPUT_BYTES_USIZE == FACADE_MAX_OUTPUT_BYTES_USIZE);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorStatus {
    ProbePassed,
    ProbeFailed,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DoctorReport {
    pub backend: &'static str,
    pub status: DoctorStatus,
    pub supported_target: bool,
    pub runtime_ready: bool,
    pub production_ready: bool,
    pub runtime_version: Option<String>,
    pub reason: &'static str,
}

impl DoctorReport {
    #[must_use]
    pub const fn passed(&self) -> bool {
        matches!(self.status, DoctorStatus::ProbePassed)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct VersionReport {
    pub version: &'static str,
    pub envelope_schema: &'static str,
    pub production_ready: bool,
    pub macos_development_lifecycle: CapabilityState,
    pub native_kvm_lifecycle: CapabilityState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Compiled,
    Unavailable,
}

#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct CommandReport {
    pub instance_id: InstanceId,
    pub execution: CommandStatus,
    pub stdout: OutputBytes,
    pub stderr: OutputBytes,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MachineReport {
    pub instance_id: InstanceId,
    pub state: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InspectionReport {
    pub instance_id: InstanceId,
    pub state: MachineState,
    pub backend: BackendKind,
}

#[derive(Clone, Eq, PartialEq)]
pub struct OutputBytes(Box<[u8]>);

impl OutputBytes {
    #[must_use]
    pub fn new(bytes: &[u8]) -> Self {
        Self(Box::from(bytes))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Debug for OutputBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OutputBytes")
            .field("bytes", &self.len())
            .finish()
    }
}

impl Serialize for OutputBytes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.len() > MAX_OUTPUT_BYTES_USIZE {
            return Err(serde::ser::Error::custom(
                "guest output exceeds the portable allowance",
            ));
        }
        let mut output = serializer.serialize_struct("EncodedBytes", 3)?;
        output.serialize_field("encoding", "base64")?;
        output.serialize_field("byte_length", &self.len())?;
        output.serialize_field("data", &STANDARD.encode(self.as_bytes()))?;
        output.end()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct FailureBody {
    pub code: &'static str,
    pub message: &'static str,
    pub retryable: bool,
}

impl FailureBody {
    #[must_use]
    pub const fn new(code: &'static str, message: &'static str, retryable: bool) -> Self {
        Self {
            code,
            message,
            retryable,
        }
    }

    #[must_use]
    pub const fn usage() -> Self {
        Self::new("usage", "command line validation failed; use --help", false)
    }

    #[must_use]
    pub const fn invalid(reason: &'static str) -> Self {
        Self::new("invalid_input", reason, false)
    }
}

#[derive(Clone, Serialize)]
#[serde(untagged)]
pub enum ResultBody {
    Version(VersionReport),
    Doctor(DoctorReport),
    Command(CommandReport),
    Machine(MachineReport),
    Inspection(InspectionReport),
    File(FileReport),
    Pty(PtyReport),
    List(SandboxListReport),
}

pub struct Response {
    command: &'static str,
    result: Option<ResultBody>,
    error: Option<FailureBody>,
    receipt: Option<ExecutionReceipt>,
}

impl Response {
    #[must_use]
    pub const fn success(command: &'static str, result: ResultBody) -> Self {
        Self {
            command,
            result: Some(result),
            error: None,
            receipt: None,
        }
    }

    #[must_use]
    pub const fn with_receipt(
        command: &'static str,
        result: ResultBody,
        receipt: ExecutionReceipt,
        error: Option<FailureBody>,
    ) -> Self {
        Self {
            command,
            result: Some(result),
            error,
            receipt: Some(receipt),
        }
    }

    #[must_use]
    pub const fn failure(command: &'static str, error: FailureBody) -> Self {
        Self {
            command,
            result: None,
            error: Some(error),
            receipt: None,
        }
    }

    /// A failure that still carries the result document describing it.
    ///
    /// A filesystem refusal is the case this exists for: the operation reached the guest and the
    /// guest declined, so the typed cause is in the result and the envelope still reports error.
    #[must_use]
    pub const fn failure_with_result(
        command: &'static str,
        result: ResultBody,
        error: FailureBody,
    ) -> Self {
        Self {
            command,
            result: Some(result),
            error: Some(error),
            receipt: None,
        }
    }

    #[must_use]
    pub const fn failure_with_receipt(
        command: &'static str,
        result: Option<ResultBody>,
        error: FailureBody,
        receipt: ExecutionReceipt,
    ) -> Self {
        Self {
            command,
            result,
            error: Some(error),
            receipt: Some(receipt),
        }
    }

    #[must_use]
    pub const fn command(&self) -> &'static str {
        self.command
    }

    #[must_use]
    pub const fn result(&self) -> Option<&ResultBody> {
        self.result.as_ref()
    }

    #[must_use]
    pub const fn error(&self) -> Option<&FailureBody> {
        self.error.as_ref()
    }

    #[must_use]
    pub const fn receipt(&self) -> Option<&ExecutionReceipt> {
        self.receipt.as_ref()
    }

    #[must_use]
    pub const fn status(&self) -> &'static str {
        if self.error.is_some() { "error" } else { "ok" }
    }

    #[must_use]
    pub fn output_is_within_declared_bound(&self) -> bool {
        let Some(ResultBody::Command(report)) = self.result() else {
            return true;
        };
        report
            .stdout
            .len()
            .checked_add(report.stderr.len())
            .is_some_and(|bytes| bytes <= MAX_OUTPUT_BYTES_USIZE)
    }
}
