use std::future::Future;

use crate::{
    BackendTarget, CommandResult, DestroyRequest, DoctorReport, ExecRequest, InspectRequest,
    InspectResult, LaunchRequest, MachineResult, RunRequest, StopRequest,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeRequest {
    Doctor { backend: BackendTarget },
    Run(RunRequest),
    Launch(LaunchRequest),
    Exec(ExecRequest),
    File(crate::FileRequest),
    Terminal(crate::TerminalRequest),
    List { backend: BackendTarget },
    Inspect(InspectRequest),
    Stop(StopRequest),
    Destroy(DestroyRequest),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeResponse {
    Doctor(DoctorReport),
    Run(CommandResult),
    Launch(MachineResult),
    Exec(CommandResult),
    File(crate::FileResult),
    Terminal(crate::TerminalResult),
    List(crate::ListResult),
    Inspect(InspectResult),
    Stop(MachineResult),
    Destroy(MachineResult),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeFailureKind {
    Unsupported,
    Unavailable,
    Rejected,
    InvalidState,
    NotFound,
    Conflict,
    Timeout,
    OutputLimit,
    CleanupIncomplete,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeFailure {
    kind: RuntimeFailureKind,
    receipt: Option<crate::ExecutionReceipt>,
    command: Option<FailureCommandEvidence>,
}

impl RuntimeFailure {
    #[must_use]
    pub const fn new(kind: RuntimeFailureKind) -> Self {
        Self {
            kind,
            receipt: None,
            command: None,
        }
    }

    #[must_use]
    pub const fn with_receipt(kind: RuntimeFailureKind, receipt: crate::ExecutionReceipt) -> Self {
        Self {
            kind,
            receipt: Some(receipt),
            command: None,
        }
    }

    #[must_use]
    pub const fn with_command_evidence(
        kind: RuntimeFailureKind,
        receipt: crate::ExecutionReceipt,
        status: crate::CommandStatus,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    ) -> Self {
        Self {
            kind,
            receipt: Some(receipt),
            command: Some(FailureCommandEvidence {
                status,
                stdout,
                stderr,
            }),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> RuntimeFailureKind {
        self.kind
    }

    pub(crate) const fn receipt(&self) -> Option<&crate::ExecutionReceipt> {
        self.receipt.as_ref()
    }

    pub(crate) const fn command(&self) -> Option<&FailureCommandEvidence> {
        self.command.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FailureCommandEvidence {
    status: crate::CommandStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl FailureCommandEvidence {
    pub(crate) const fn status(&self) -> crate::CommandStatus {
        self.status
    }

    pub(crate) fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    pub(crate) fn stderr(&self) -> &[u8] {
        &self.stderr
    }
}

pub trait ToolRuntime: Send + Sync + 'static {
    fn invoke(
        &self,
        request: RuntimeRequest,
    ) -> impl Future<Output = Result<RuntimeResponse, RuntimeFailure>> + Send;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UnavailableRuntime;

impl ToolRuntime for UnavailableRuntime {
    fn invoke(
        &self,
        request: RuntimeRequest,
    ) -> impl Future<Output = Result<RuntimeResponse, RuntimeFailure>> + Send {
        std::future::ready(match request {
            RuntimeRequest::Doctor { backend } => Ok(RuntimeResponse::Doctor(DoctorReport {
                backend,
                status: crate::DoctorStatus::Unsupported,
                supported_target: false,
                runtime_ready: false,
                production_ready: false,
            })),
            RuntimeRequest::Run(_)
            | RuntimeRequest::Launch(_)
            | RuntimeRequest::Exec(_)
            | RuntimeRequest::File(_)
            | RuntimeRequest::Terminal(_)
            | RuntimeRequest::List { .. }
            | RuntimeRequest::Inspect(_)
            | RuntimeRequest::Stop(_)
            | RuntimeRequest::Destroy(_) => {
                Err(RuntimeFailure::new(RuntimeFailureKind::Unavailable))
            }
        })
    }
}
