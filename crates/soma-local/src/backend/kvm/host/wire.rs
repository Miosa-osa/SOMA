//! The bounded JSON a client and its resident machine host exchange.
//!
//! Nothing here is a portable observation. The host reports the facts it established and the
//! client builds the observation from them together with the request it already holds, so the
//! evidence a caller reads is still assembled on the side that was asked for it, timed by that
//! side's own clock.

use serde::{Deserialize, Serialize};
use soma::{
    BackendFailureKind, CleanupEvidence, CommandStatus, EffectiveNetwork, FileAnswer,
    FileOperation, GenerationId, InstanceId, MachineShape, MachineState, OperationId,
    PreparationClass, PtyAnswer, PtyOperation,
};
use std::path::PathBuf;

/// The largest line either side will read, so neither can spend the other's memory.
pub(super) const MAX_LINE_BYTES: u64 = 64 << 20;

/// Generation and shape facts an identity-free host consumes before request traffic begins.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrewarmWire {
    pub(super) reference: String,
    pub(super) generation_id: GenerationId,
    pub(super) manifest: Vec<u8>,
    pub(super) memory_mib: u64,
}

/// The first message after the verified descriptor handoff.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum InitialWire {
    /// Restore one identity-free machine, then wait for its later launch assignment.
    Prewarm(PrewarmWire),
    /// Launch immediately through the on-demand path.
    Launch(Box<LaunchWire>),
}

/// Proof that a child completed identity-free restore before it entered the available pool.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PrewarmReady {
    Prepared,
    Refused,
}

/// Everything the host needs to build the one machine it will ever hold.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LaunchWire {
    /// Address the child binds only after it receives the complete launch capability.
    pub(super) socket: PathBuf,
    pub(super) operation_id: OperationId,
    pub(super) instance_id: InstanceId,
    /// The image reference the client resolved.
    ///
    /// The host finds its own prepared entry from this rather than being handed a store path, so
    /// what it launches is what a prepared entry claims rather than bytes the client named.
    pub(super) reference: String,
    /// Identity of the manifest whose verified files cross on the launch channel.
    pub(super) generation_id: GenerationId,
    /// Canonical ready manifest bytes already admitted by the parent.
    pub(super) manifest: Vec<u8>,
    pub(super) shape: MachineShape,
}

/// What a launch established, whichever side performed it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::backend::kvm) struct Launched {
    pub(in crate::backend::kvm) preparation: PreparationClass,
    pub(in crate::backend::kvm) memory_mib: u64,
    pub(in crate::backend::kvm) storage_mib: u64,
    pub(in crate::backend::kvm) network: EffectiveNetwork,
    /// When the machine existed, in nanoseconds after its launch was admitted.
    pub(in crate::backend::kvm) at_ns: u64,
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
    /// Perform one bounded filesystem operation inside the machine this host holds.
    ///
    /// The portable operation crosses as itself rather than as a guest protocol body, because
    /// the host rebuilds the guest request from it with exactly the mapping the resident path
    /// uses. Relaying a pre-built body would let a client choose a request the mapping would not
    /// have produced, including one naming a path this side never meant to allow.
    File {
        instance_id: InstanceId,
        operation: FileOperation,
    },
    /// Perform one bounded terminal operation inside the machine this host holds.
    ///
    /// The portable operation crosses as itself for the reason the filesystem one does: the host
    /// rebuilds the guest request from it with exactly the mapping the resident path uses, so a
    /// client cannot choose a request that mapping would not have produced.
    Pty {
        instance_id: InstanceId,
        operation: PtyOperation,
    },
    Inspect {
        instance_id: InstanceId,
    },
    Cleanup {
        instance_id: InstanceId,
        /// Whether the machine may be ended without asking the guest.
        forced: bool,
    },
    /// End the host, because nothing has addressed its machine for long enough.
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
    FileAnswered {
        answer: FileAnswer,
    },
    PtyAnswered {
        answer: PtyAnswer,
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
