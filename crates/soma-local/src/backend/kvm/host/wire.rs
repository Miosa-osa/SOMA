//! The bounded JSON a client and its resident machine host exchange.
//!
//! Nothing here is a portable observation. The host reports the facts it established and the
//! client builds the observation from them together with the request it already holds, so the
//! evidence a caller reads is still assembled on the side that was asked for it.

use serde::{Deserialize, Serialize};
use soma::{
    BackendFailureKind, CleanupEvidence, CommandStatus, EffectiveNetwork, InstanceId, MachineShape,
    MachineState, OperationId, PreparationClass,
};

/// The largest line either side will read, so neither can spend the other's memory.
pub(super) const MAX_LINE_BYTES: u64 = 64 << 20;

/// Everything the host needs to build the one machine it will ever hold.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LaunchWire {
    pub(super) operation_id: OperationId,
    pub(super) instance_id: InstanceId,
    /// The image reference the client resolved.
    ///
    /// The host finds its own prepared entry from this rather than being handed a path, so what
    /// it launches is what a prepared entry claims rather than bytes the client named.
    pub(super) reference: String,
    pub(super) shape: MachineShape,
}

/// What a launch established, whichever side performed it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Launched {
    pub(crate) preparation: PreparationClass,
    pub(crate) memory_mib: u64,
    pub(crate) network: EffectiveNetwork,
    /// When the machine existed, in nanoseconds after the launch was admitted.
    pub(crate) launched_ns: u64,
}

/// The one line a host writes to its standard output before it stops using it.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Ready {
    Launched(Launched),
    Refused(Refusal),
}

/// What a later process asks the host holding its machine to do.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Call {
    Execute {
        instance_id: InstanceId,
        program: Vec<u8>,
        arguments: Vec<Vec<u8>>,
        timeout_ms: u32,
        max_output_bytes: u64,
    },
    Inspect {
        instance_id: InstanceId,
    },
    Cleanup {
        instance_id: InstanceId,
    },
    /// End the host because nothing has addressed it for long enough.
    Shutdown,
}

/// What the host answers.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Answer {
    Executed {
        status: CommandStatus,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    Inspected {
        state: MachineState,
        network: EffectiveNetwork,
    },
    Cleaned {
        evidence: CleanupEvidence,
    },
    Refused(Refusal),
}

/// A backend refusal in a form that crosses a process boundary.
///
/// `BackendFailureKind` carries no timing, and the timing a caller reads belongs to the clock of
/// the process it asked, so only the kind travels.
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Refusal {
    Unsupported,
    Unavailable,
    ResourceConflict,
    WorkloadRejected,
    IsolationFailure,
    GuestFailure,
    Timeout,
    OutputLimit,
    CleanupFailure,
}

impl From<BackendFailureKind> for Refusal {
    fn from(kind: BackendFailureKind) -> Self {
        match kind {
            BackendFailureKind::Unsupported => Self::Unsupported,
            BackendFailureKind::Unavailable => Self::Unavailable,
            BackendFailureKind::ResourceConflict => Self::ResourceConflict,
            BackendFailureKind::WorkloadRejected => Self::WorkloadRejected,
            BackendFailureKind::IsolationFailure => Self::IsolationFailure,
            BackendFailureKind::GuestFailure => Self::GuestFailure,
            BackendFailureKind::Timeout => Self::Timeout,
            BackendFailureKind::OutputLimit => Self::OutputLimit,
            BackendFailureKind::CleanupFailure => Self::CleanupFailure,
        }
    }
}

impl From<Refusal> for BackendFailureKind {
    fn from(refusal: Refusal) -> Self {
        match refusal {
            Refusal::Unsupported => Self::Unsupported,
            Refusal::Unavailable => Self::Unavailable,
            Refusal::ResourceConflict => Self::ResourceConflict,
            Refusal::WorkloadRejected => Self::WorkloadRejected,
            Refusal::IsolationFailure => Self::IsolationFailure,
            Refusal::GuestFailure => Self::GuestFailure,
            Refusal::Timeout => Self::Timeout,
            Refusal::OutputLimit => Self::OutputLimit,
            Refusal::CleanupFailure => Self::CleanupFailure,
        }
    }
}
