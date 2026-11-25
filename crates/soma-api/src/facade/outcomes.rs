//! What the facade's operations answer with.

use soma::{
    BackendKind, ExecutionReceipt, FileAnswer, InstanceId, MachineState, PtyAnswer, TerminalStatus,
};

/// One managed lifecycle transition, carrying the receipt that proves it happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecycleOutcome {
    pub instance_id: InstanceId,
    pub receipt: ExecutionReceipt,
}

/// One command execution and the guest output it produced.
///
/// Output is carried as raw bytes rather than a string because guest output is not required to be
/// UTF-8, and the wire layer base64 encodes it exactly as the CLI does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutcome {
    pub instance_id: InstanceId,
    pub status: TerminalStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub receipt: ExecutionReceipt,
}

/// One completed filesystem operation and what the guest answered.
///
/// There is no receipt here because the operation mints none: it runs no program and leaves
/// nothing a later caller could replay, so a receipt would have to be invented to carry it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileOutcome {
    pub instance_id: InstanceId,
    pub operation: &'static str,
    pub answer: FileAnswer,
}

/// One completed terminal operation and what the guest answered.
///
/// There is no receipt here for the reason a filesystem operation has none: the operation mints
/// one, and inventing an empty one to fill the field would be evidence of nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalOutcome {
    pub instance_id: InstanceId,
    pub operation: &'static str,
    pub answer: PtyAnswer,
}

/// The observed state of one managed sandbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxSnapshot {
    pub instance_id: InstanceId,
    pub state: MachineState,
    pub backend: BackendKind,
    pub receipt: ExecutionReceipt,
    /// The machine is gone without a stop: its worker died, or a command took it down.
    pub machine_lost: bool,
}

/// The answer of a facade that cannot suspend: the sandbox keeps running.
pub(super) const fn unsupported_suspension() -> soma_local::SuspensionFailure {
    soma_local::SuspensionFailure::Managed(soma::ManagedFailure::Backend(
        soma::BackendFailureKind::Unsupported,
    ))
}
