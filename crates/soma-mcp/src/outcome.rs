use serde::Serialize;

use crate::{BackendTarget, InstanceId};

const MAX_RECEIPT_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorStatus {
    ProbePassed,
    ProbeFailed,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    pub backend: BackendTarget,
    pub status: DoctorStatus,
    pub supported_target: bool,
    pub runtime_ready: bool,
    pub production_ready: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandStatus {
    Exited { code: i32 },
    Signaled { signal: Option<i32> },
    TimedOut,
    OutputLimitExceeded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionReceipt(serde_json::Value);

impl ExecutionReceipt {
    /// Accepts an opaque facade receipt while retaining a strict MCP size bound.
    ///
    /// # Errors
    ///
    /// Returns [`ReceiptValidationError`] when `value` is not an object, cannot
    /// be encoded as JSON, or exceeds 256 KiB.
    pub fn new(value: serde_json::Value) -> Result<Self, ReceiptValidationError> {
        if !value.is_object()
            || serde_json::to_vec(&value).map_or(true, |bytes| bytes.len() > MAX_RECEIPT_BYTES)
        {
            return Err(ReceiptValidationError);
        }
        Ok(Self(value))
    }

    pub(crate) const fn as_value(&self) -> &serde_json::Value {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReceiptValidationError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandResult {
    instance_id: InstanceId,
    status: CommandStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    receipt: ExecutionReceipt,
}

impl CommandResult {
    #[must_use]
    pub const fn new(
        instance_id: InstanceId,
        status: CommandStatus,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        receipt: ExecutionReceipt,
    ) -> Self {
        Self {
            instance_id,
            status,
            stdout,
            stderr,
            receipt,
        }
    }

    pub(crate) const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    pub(crate) const fn status(&self) -> CommandStatus {
        self.status
    }

    pub(crate) fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    pub(crate) fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    pub(crate) const fn receipt(&self) -> &ExecutionReceipt {
        &self.receipt
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineState {
    Starting,
    Ready,
    Running,
    Stopping,
    Stopped,
    Destroyed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineResult {
    instance_id: InstanceId,
    state: MachineState,
    receipt: ExecutionReceipt,
}

impl MachineResult {
    #[must_use]
    pub const fn new(
        instance_id: InstanceId,
        state: MachineState,
        receipt: ExecutionReceipt,
    ) -> Self {
        Self {
            instance_id,
            state,
            receipt,
        }
    }

    pub(crate) const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    pub(crate) const fn state(&self) -> MachineState {
        self.state
    }

    pub(crate) const fn receipt(&self) -> &ExecutionReceipt {
        &self.receipt
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectResult {
    instance_id: InstanceId,
    state: MachineState,
    backend: BackendTarget,
    receipt: Option<ExecutionReceipt>,
}

impl InspectResult {
    #[must_use]
    pub const fn new(
        instance_id: InstanceId,
        state: MachineState,
        backend: BackendTarget,
        receipt: Option<ExecutionReceipt>,
    ) -> Self {
        Self {
            instance_id,
            state,
            backend,
            receipt,
        }
    }

    pub(crate) const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    pub(crate) const fn state(&self) -> MachineState {
        self.state
    }

    pub(crate) const fn backend(&self) -> BackendTarget {
        self.backend
    }

    pub(crate) const fn receipt(&self) -> Option<&ExecutionReceipt> {
        self.receipt.as_ref()
    }
}
