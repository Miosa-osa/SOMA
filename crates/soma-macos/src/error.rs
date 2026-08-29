use std::{error::Error, fmt};

use serde::Serialize;

use crate::{ExecutionStatus, InstanceId, PublishedPort};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    ProbeVersion,
    ProbeStatus,
    PullImage,
    InspectImage,
    Run,
    Create,
    Start,
    Execute,
    Stop,
    Delete,
    Inspect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageResolutionFailure {
    InvalidJson,
    MissingImageRecord,
    MultipleImageRecords,
    MissingVariant,
    MultipleVariants,
    PlatformMismatch,
    MalformedIndexDigest,
    MalformedManifestDigest,
    IndexIdentityMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipFailure {
    InvalidJson,
    MissingRecord,
    MultipleRecords,
    MalformedRecord,
    NameMismatch,
    MissingLabel,
    LabelMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessFailureKind {
    ExecutableUnavailable,
    PermissionDenied,
    SpawnFailed,
    PipeUnavailable,
    ReadFailed,
    WaitFailed,
    KillFailed,
    ReaderPanicked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum CommandFailureReason {
    Process(ProcessFailureKind),
    Status(ExecutionStatus),
    Ownership(OwnershipFailure),
    InvalidJson,
    MissingVersionComponent,
    RuntimeNotRunning,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct CommandFailure {
    operation: Operation,
    reason: CommandFailureReason,
}

impl CommandFailure {
    pub(crate) const fn new(operation: Operation, reason: CommandFailureReason) -> Self {
        Self { operation, reason }
    }

    #[must_use]
    pub const fn operation(self) -> Operation {
        self.operation
    }

    #[must_use]
    pub const fn reason(self) -> CommandFailureReason {
        self.reason
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BackendError {
    UnsupportedHost,
    UnsupportedVersion {
        found: String,
        supported: &'static str,
    },
    ImageResolution {
        failure: ImageResolutionFailure,
    },
    Command {
        failure: CommandFailure,
    },
    CleanupFailed {
        instance_id: InstanceId,
        primary_failed: bool,
        cleanup: CommandFailure,
    },
    ManagedExecutionInvalidated {
        instance_id: InstanceId,
        failure: CommandFailure,
        cleanup: crate::CleanupState,
        cleanup_published_ports: Option<Vec<PublishedPort>>,
    },
}

impl BackendError {
    pub(crate) const fn command(failure: CommandFailure) -> Self {
        Self::Command { failure }
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedHost => {
                formatter.write_str("macOS virtualization backend unavailable")
            }
            Self::UnsupportedVersion { .. } => {
                formatter.write_str("Apple container CLI version is unsupported")
            }
            Self::ImageResolution { .. } => {
                formatter.write_str("image identity resolution failed closed")
            }
            Self::Command { failure } => {
                write!(
                    formatter,
                    "backend command failed during {:?}",
                    failure.operation
                )
            }
            Self::CleanupFailed { .. } => {
                formatter.write_str("sandbox cleanup could not be proven")
            }
            Self::ManagedExecutionInvalidated { .. } => {
                formatter.write_str("managed execution failed and its sandbox was invalidated")
            }
        }
    }
}

impl Error for BackendError {}
