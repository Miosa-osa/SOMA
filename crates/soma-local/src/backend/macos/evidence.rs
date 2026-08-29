use soma::{
    AssignedAddress, BackendFailureKind, CommandStatus, EffectiveNetwork, EffectiveShape,
    MachineState, NetworkAttachment, Observation, ObservationUnavailable,
};
use soma_macos::{ExecutionStatus, NetworkAttachment as MacNetworkAttachment};

use super::network::ActivationExpectation;

pub(super) fn launch_evidence(
    requested: &soma::MachineShape,
    inspection: &soma_macos::InspectedMachine,
    network_expectation: &ActivationExpectation,
) -> Result<(EffectiveShape, EffectiveNetwork), BackendFailureKind> {
    let unavailable_vcpus = Observation::Unavailable(ObservationUnavailable::NotVerified);
    let unavailable_memory = Observation::Unavailable(ObservationUnavailable::NotVerified);
    let (vcpus, memory) = inspection.resources().map_or_else(
        || (unavailable_vcpus, unavailable_memory),
        |resources| {
            let memory_mib = resources.memory_bytes() / 1_048_576;
            (
                Observation::Observed(resources.vcpus()),
                Observation::Observed(memory_mib),
            )
        },
    );
    if matches!(&vcpus, Observation::Observed(value) if *value != requested.vcpu_count())
        || matches!(&memory, Observation::Observed(value) if *value != requested.memory_mib())
    {
        return Err(BackendFailureKind::IsolationFailure);
    }
    let shape = EffectiveShape::new(
        vcpus,
        memory,
        Observation::Unavailable(ObservationUnavailable::NotVerified),
    )
    .map_err(|_| BackendFailureKind::IsolationFailure)?;
    let network = effective_network(
        requested.capabilities().network_policy(),
        inspection,
        network_expectation,
    )?;
    Ok((shape, network))
}

pub(super) fn effective_network(
    policy: &soma::NetworkPolicy,
    inspection: &soma_macos::InspectedMachine,
    network_expectation: &ActivationExpectation,
) -> Result<EffectiveNetwork, BackendFailureKind> {
    let (attachment, egress) = match (policy.egress(), inspection.network_attachment()) {
        (soma::EgressPolicy::Denied, Some(MacNetworkAttachment::Detached)) => (
            Observation::Observed(NetworkAttachment::Detached),
            Observation::Observed(soma::EgressPolicy::Denied),
        ),
        (soma::EgressPolicy::Unrestricted, Some(MacNetworkAttachment::Attached)) => (
            Observation::Observed(NetworkAttachment::Attached),
            Observation::Observed(soma::EgressPolicy::Unrestricted),
        ),
        (soma::EgressPolicy::Unspecified, Some(MacNetworkAttachment::Detached)) => (
            Observation::Observed(NetworkAttachment::Detached),
            Observation::Unavailable(ObservationUnavailable::NotVerified),
        ),
        (soma::EgressPolicy::Unspecified, Some(MacNetworkAttachment::Attached)) => (
            Observation::Observed(NetworkAttachment::Attached),
            Observation::Unavailable(ObservationUnavailable::NotVerified),
        ),
        (soma::EgressPolicy::Unspecified, None) => (
            Observation::Unavailable(ObservationUnavailable::NotVerified),
            Observation::Unavailable(ObservationUnavailable::NotVerified),
        ),
        (
            soma::EgressPolicy::Denied
            | soma::EgressPolicy::PublicInternet
            | soma::EgressPolicy::Unrestricted,
            _,
        ) => return Err(BackendFailureKind::IsolationFailure),
    };
    let dns = match policy.dns() {
        soma::DnsPolicy::Denied if policy.egress() == soma::EgressPolicy::Denied => {
            Observation::Observed(soma::DnsPolicy::Denied)
        }
        soma::DnsPolicy::Unspecified => {
            Observation::Unavailable(ObservationUnavailable::NotVerified)
        }
        soma::DnsPolicy::Custom { servers }
            if inspection.network().dns_servers() == Some(servers.as_slice()) =>
        {
            Observation::Observed(policy.dns().clone())
        }
        soma::DnsPolicy::Denied | soma::DnsPolicy::System | soma::DnsPolicy::Custom { .. } => {
            return Err(BackendFailureKind::IsolationFailure);
        }
    };
    let addresses = inspection.network().addresses().map_or_else(
        || Observation::Unavailable(ObservationUnavailable::NotVerified),
        |values| {
            values
                .iter()
                .map(|value| AssignedAddress::new(value.address(), value.prefix_length()))
                .collect::<Result<Vec<_>, _>>()
                .map(Observation::Observed)
                .unwrap_or_else(|_| Observation::Unavailable(ObservationUnavailable::NotVerified))
        },
    );
    EffectiveNetwork::new(
        attachment,
        egress,
        dns,
        addresses,
        Observation::Observed(network_expectation.publications().to_vec()),
        Observation::Observed(network_expectation.activation()),
    )
    .map_err(|_| BackendFailureKind::IsolationFailure)
}

pub(super) fn inspection_state(inspection: &soma_macos::InspectedMachine) -> Option<MachineState> {
    let [record] = inspection.document().as_array()?.as_slice() else {
        return None;
    };
    match record.get("status")?.get("state")?.as_str()? {
        "running" => Some(MachineState::Ready),
        "stopping" => Some(MachineState::Stopping),
        _ => None,
    }
}

pub(super) const fn command_status(status: ExecutionStatus) -> CommandStatus {
    match status {
        ExecutionStatus::Exited { code } => CommandStatus::Exited { code },
        ExecutionStatus::Signaled => CommandStatus::Signaled { signal: None },
        ExecutionStatus::TimedOut => CommandStatus::TimedOut,
        ExecutionStatus::OutputLimitExceeded => CommandStatus::OutputLimitExceeded,
    }
}
