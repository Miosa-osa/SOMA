//! The bounded JSON a client and its resident machine host exchange.
//!
//! Nothing here is a portable observation. The host reports the facts it established and the
//! client builds the observation from them together with the request it already holds, so the
//! evidence a caller reads is still assembled on the side that was asked for it, timed by that
//! side's own clock.

pub(super) mod exchange;

pub(super) use exchange::{Answer, Call};

use serde::{Deserialize, Serialize};
use soma::{
    BackendFailureKind, EffectiveNetwork, GenerationId, InstanceId, MachineShape, OperationId,
    PreparationClass,
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
    /// The durable volume this Instance binds, when its Generation declares one.
    ///
    /// The path is the host's own, named by the volume store after it certified the file's
    /// mount, the volume's state, and this start's repair. It is written only when set, so a
    /// host that predates the volume still reads every launch, and a host that predates it
    /// reads a frame carrying one as unknown rather than ignoring it, which is the refusal this
    /// machine wants: a launch it cannot bind must not run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) volume: Option<soma::VolumeAttachment>,
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
    /// The parent saw too many launches in flight for this one to run the warm command.
    ///
    /// It is written only when set, so a host that predates it still reads every launch that
    /// warms, and read as unset when absent, so a parent that predates it still warms.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) skip_warm: bool,
}

/// What a launch established, whichever side performed it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::backend::kvm) struct Launched {
    pub(in crate::backend::kvm) preparation: PreparationClass,
    pub(in crate::backend::kvm) memory_mib: u64,
    /// The capacity of the private block backend whose agreement with the captured device
    /// state was checked before the guest could reach Ready.
    pub(in crate::backend::kvm) storage_mib: u64,
    /// The vCPU count the machine was actually built with.
    pub(in crate::backend::kvm) vcpus: u16,
    pub(in crate::backend::kvm) network: EffectiveNetwork,
    /// When the machine existed, in nanoseconds after its launch was admitted.
    pub(in crate::backend::kvm) at_ns: u64,
    /// Whether the host left the API unit's control group, when a live subtree is configured.
    ///
    /// Absent from the line when none is, so a host built before this field still reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::backend::kvm) live_cgroup: Option<LiveCgroup>,
}

/// What a claimed host reports about leaving its unit's control group.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(in crate::backend::kvm) struct LiveCgroup {
    /// The host is in its own leaf, so it survives a stop of the API unit.
    pub(in crate::backend::kvm) joined: bool,
    /// The leaf is accounted for memory and pids, under the subtree root's limit.
    pub(in crate::backend::kvm) accounted: bool,
}

/// The one line a host writes to its standard output before it stops using it.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Ready {
    Launched(Launched),
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

#[cfg(test)]
mod context_tests {
    use super::Call;
    use crate::backend::kvm::evidence::WireContext;

    #[test]
    fn a_context_crosses_the_host_channel_as_its_own_call() {
        let context = WireContext {
            environment: vec![(b"A".to_vec(), b"1".to_vec())],
            working_directory: Some(b"/work".to_vec()),
            user: None,
            stdin: b"in".to_vec(),
        };
        let call = Call::ExecuteInContext {
            instance_id: soma::InstanceId::new("22222222222222222222222222222222").expect("id"),
            program: b"/bin/true".to_vec(),
            arguments: Vec::new(),
            timeout_ms: 1_000,
            max_output_bytes: 1_024,
            context: context.clone(),
        };
        let line = serde_json::to_string(&call).expect("encodes");
        assert!(line.contains("execute_in_context"));
        match serde_json::from_str::<Call>(&line).expect("decodes") {
            Call::ExecuteInContext { context: got, .. } => assert!(got == context),
            _ => panic!("the call changed variant on the way"),
        }
    }

    /// A host built before contexts existed names no such variant, so it refuses the line
    /// instead of running the command without the environment the caller asked for. This is the
    /// property the separate variant exists to keep: the plain `Execute` shape never gains a
    /// field an old host would ignore.
    #[test]
    fn the_plain_execute_call_carries_no_context_field() {
        let call = Call::Execute {
            instance_id: soma::InstanceId::new("22222222222222222222222222222222").expect("id"),
            program: b"/bin/true".to_vec(),
            arguments: Vec::new(),
            timeout_ms: 1_000,
            max_output_bytes: 1_024,
        };
        let line = serde_json::to_string(&call).expect("encodes");
        assert!(!line.contains("context") && !line.contains("environment"));
    }
}
