//! What a caller is told it received, and how a guest result becomes a portable one.

use soma::{
    CommandObservation, CommandStatus, CommandTimes, DnsPolicy, EffectiveNetwork, EffectiveShape,
    EgressPolicy, ExecutionRequest, NetworkAttachment, NetworkPolicy, Observation,
    ObservationUnavailable, ObservedOutput, PortActivationClass,
};
use soma_guest::TerminalStatus;

use super::network::Egress;

/// The one machine shape the `x86_64` contract admits.
pub(super) const CONTRACT_VCPUS: u16 = 1;

/// The shape a caller is told it received.
///
/// The vCPU count is fixed by the machine contract and the memory is the amount the machine was
/// built with, so both are observed. Storage is not measured by this Backend.
pub(super) fn effective_shape(memory_mib: u64) -> EffectiveShape {
    EffectiveShape::new(
        Observation::Observed(CONTRACT_VCPUS),
        Observation::Observed(memory_mib),
        Observation::Unavailable(ObservationUnavailable::NotVerified),
    )
    .unwrap_or_else(|_| {
        EffectiveShape::new(
            Observation::Unavailable(ObservationUnavailable::NotVerified),
            Observation::Unavailable(ObservationUnavailable::NotVerified),
            Observation::Unavailable(ObservationUnavailable::NotVerified),
        )
        .expect("an entirely unavailable shape is always valid")
    })
}

/// The network a caller is told it received.
///
/// Every dimension is observed rather than unverified, because each is a fact this process
/// established: an Instance that asked for no egress holds the link-down device and reaches
/// nothing, and an Instance the broker leased a bundle to holds exactly the address the broker
/// leased under exactly the policy the broker admitted. Reporting a request's policy back as
/// effective would describe what was asked for rather than what was given.
pub(super) fn effective_network(egress: &Egress, policy: &NetworkPolicy) -> EffectiveNetwork {
    let observed = match egress {
        Egress::Declined => EffectiveNetwork::new(
            Observation::Observed(NetworkAttachment::Detached),
            Observation::Observed(EgressPolicy::Denied),
            Observation::Observed(DnsPolicy::Denied),
            Observation::Observed(Vec::new()),
            Observation::Observed(Vec::new()),
            Observation::Observed(PortActivationClass::NotApplicable),
        ),
        Egress::Leased(lease) => EffectiveNetwork::new(
            Observation::Observed(NetworkAttachment::Attached),
            Observation::Observed(policy.egress()),
            Observation::Observed(policy.dns().clone()),
            Observation::Observed(lease.addresses()),
            Observation::Observed(Vec::new()),
            Observation::Observed(PortActivationClass::NotApplicable),
        ),
    };
    observed.unwrap_or_else(|_| EffectiveNetwork::unavailable(ObservationUnavailable::NotVerified))
}

/// One command lowered to the values a guest agent accepts.
///
/// The lowering happens on the side that was asked, whether or not the machine is in this
/// process, so a request no guest could accept is refused before any other process is addressed.
pub(super) struct CommandParts {
    pub(super) program: Vec<u8>,
    pub(super) arguments: Vec<Vec<u8>>,
    pub(super) timeout_ms: u32,
    pub(super) max_output_bytes: u64,
}

pub(super) fn command_parts(request: &ExecutionRequest<'_>) -> Option<CommandParts> {
    let command = request.command();
    let limits = request.limits();
    Some(CommandParts {
        program: command.executable().as_bytes().to_vec(),
        arguments: command
            .arguments()
            .iter()
            .map(|argument| argument.as_bytes().to_vec())
            .collect(),
        timeout_ms: u32::try_from(limits.timeout_ms()).ok()?,
        max_output_bytes: limits.max_output_bytes(),
    })
}

/// The portable status for a command the guest actually ran.
///
/// `ExecFailed` and `AgentFailed` have no portable equivalent, and they are not command results:
/// the first means the program never started, the second that the agent itself failed. Reporting
/// either as an exit code would describe a command that never ran as one that ran and finished,
/// so they become a guest failure instead.
pub(super) const fn command_status(status: TerminalStatus) -> Option<CommandStatus> {
    match status {
        TerminalStatus::Exited(code) => Some(CommandStatus::Exited { code }),
        TerminalStatus::Signaled(signal) => Some(CommandStatus::Signaled {
            signal: Some(signal as i32),
        }),
        TerminalStatus::TimedOut => Some(CommandStatus::TimedOut),
        TerminalStatus::OutputLimit => Some(CommandStatus::OutputLimitExceeded),
        TerminalStatus::ExecFailed(_) | TerminalStatus::AgentFailed(_) => None,
    }
}

pub(super) fn observation(
    request: &ExecutionRequest<'_>,
    status: CommandStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    times: CommandTimes,
) -> CommandObservation {
    // The guest bounds the combined allowance and returns exactly what it kept, so the observed
    // byte counts are the lengths of the bytes themselves rather than a separate claim.
    let observed_stdout = stdout.len() as u64;
    let observed_stderr = stderr.len() as u64;
    let output = ObservedOutput::new(stdout, observed_stdout, stderr, observed_stderr);
    CommandObservation::new(
        request.operation_id().clone(),
        request.instance_id().clone(),
        status,
        output,
        times,
    )
}
