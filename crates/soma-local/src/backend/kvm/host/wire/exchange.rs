//! What a client asks a host to do, and what the host answers, over the machine socket.

use serde::{Deserialize, Serialize};
use soma::{
    CleanupEvidence, CommandStatus, EffectiveNetwork, FileAnswer, FileOperation, InstanceId,
    MachineState, PtyAnswer, PtyOperation,
};

use super::Refusal;

/// What a later process asks the host holding its machine to do.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::backend::kvm::host) enum Call {
    Execute {
        instance_id: InstanceId,
        program: Vec<u8>,
        arguments: Vec<Vec<u8>>,
        timeout_ms: u32,
        max_output_bytes: u64,
    },
    /// Run one command that carries an environment, working directory, user, or standard input.
    ///
    /// A separate variant rather than optional fields on `Execute`: a host built before contexts
    /// existed ignores a field it does not know and would run the command without the
    /// environment the caller asked for, while it refuses a variant it does not know. A command
    /// with no context is still sent as `Execute`, so an older host keeps serving it.
    ExecuteInContext {
        instance_id: InstanceId,
        program: Vec<u8>,
        arguments: Vec<Vec<u8>>,
        timeout_ms: u32,
        max_output_bytes: u64,
        context: crate::backend::kvm::evidence::WireContext,
    },
    /// Run one command and answer with its output as it is produced.
    ///
    /// The one call that gets more than one line back: any number of [`Answer::Chunk`] lines,
    /// then exactly one `Executed` or `Refused`. A host built before this call does not name the
    /// variant and refuses the line, so a streaming caller never gets a command that ran
    /// without being asked to.
    ExecuteStreaming {
        instance_id: InstanceId,
        program: Vec<u8>,
        arguments: Vec<Vec<u8>>,
        timeout_ms: u32,
        max_output_bytes: u64,
        context: crate::backend::kvm::evidence::WireContext,
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
    /// Open a byte stream to a port of the guest this host holds.
    ///
    /// The host answers with one line, and when that line is `tunneled` the connection stops
    /// being a request channel: from the next byte on it carries the guest's stream, in both
    /// directions, until either side closes it.
    Tunnel {
        instance_id: InstanceId,
        port: u16,
    },
    /// Change the CPU and memory of the machine this host holds, live.
    Resize {
        instance_id: InstanceId,
        request: crate::ResizeRequest,
    },
    /// Open or close the egress of the windowed machine this host holds.
    ///
    /// A variant of its own, so a host built before it refuses the line rather than reading it as
    /// something else and leaving the egress as it was while the caller believes it changed.
    SetEgress {
        instance_id: InstanceId,
        request: crate::EgressSetRequest,
    },
    /// End the host, because nothing has addressed its machine for long enough.
    Shutdown,
}

/// Which stream of a running command a chunk belongs to.
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::backend::kvm) enum Channel {
    Stdout,
    Stderr,
}

impl From<soma::OutputChannel> for Channel {
    fn from(channel: soma::OutputChannel) -> Self {
        match channel {
            soma::OutputChannel::Stdout => Self::Stdout,
            soma::OutputChannel::Stderr => Self::Stderr,
        }
    }
}

impl From<Channel> for soma::OutputChannel {
    fn from(channel: Channel) -> Self {
        match channel {
            Channel::Stdout => Self::Stdout,
            Channel::Stderr => Self::Stderr,
        }
    }
}

/// What the host answers.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::backend::kvm::host) enum Answer {
    Executed {
        status: CommandStatus,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    /// One chunk of a streaming command's output, before the command has ended.
    Chunk {
        channel: Channel,
        bytes: Vec<u8>,
    },
    /// The end of a streaming command, naming how many bytes crossed on each stream.
    ///
    /// The bytes themselves already arrived as chunks, so repeating them here would double the
    /// largest line either side reads.
    StreamEnded {
        status: CommandStatus,
        stdout_bytes: u64,
        stderr_bytes: u64,
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
    /// The connection now carries the guest's stream.
    Tunneled,
    Resized(crate::ResizeOutcome),
    ResizeRefused(crate::ResizeFailure),
    TunnelRefused(crate::TunnelFailure),
    EgressSet(crate::EgressSetOutcome),
    EgressRefused(crate::EgressSetFailure),
    Refused(Refusal),
}
