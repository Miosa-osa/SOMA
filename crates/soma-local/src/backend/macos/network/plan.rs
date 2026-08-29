use soma::{
    BackendFailureKind, DnsPolicy, EffectivePortPublication, EgressPolicy, HostBind, NetworkPolicy,
    PortActivationClass, TransportProtocol,
};
use soma_macos::{
    DnsConfiguration, NetworkConfiguration, NetworkPolicy as MacAttachment,
    PublishedPort as MacPublishedPort, TransportProtocol as MacProtocol,
};

use super::reservation::PortReservation;

pub(in crate::backend::macos) struct PreparedNetwork {
    configuration: NetworkConfiguration,
    reservations: Vec<PortReservation>,
    publications: Vec<EffectivePortPublication>,
}

impl PreparedNetwork {
    pub(in crate::backend::macos) fn configuration(&self) -> &NetworkConfiguration {
        &self.configuration
    }

    pub(in crate::backend::macos) fn begin_activation(self) -> ActivationExpectation {
        for reservation in self.reservations {
            reservation.release();
        }
        ActivationExpectation::observed(self.publications)
    }
}

#[derive(Clone, Debug)]
pub(in crate::backend::macos) struct ActivationExpectation {
    publications: Vec<EffectivePortPublication>,
    activation: PortActivationClass,
}

impl ActivationExpectation {
    pub(in crate::backend::macos) fn observed(publications: Vec<EffectivePortPublication>) -> Self {
        let activation = if publications.is_empty() {
            PortActivationClass::NotApplicable
        } else {
            PortActivationClass::VerifiedRuntimeRebind
        };
        Self {
            publications,
            activation,
        }
    }

    pub(in crate::backend::macos) fn publications(&self) -> &[EffectivePortPublication] {
        &self.publications
    }

    pub(in crate::backend::macos) const fn activation(&self) -> PortActivationClass {
        self.activation
    }
}

pub(in crate::backend::macos) fn prepare(
    policy: &NetworkPolicy,
) -> Result<PreparedNetwork, BackendFailureKind> {
    if !supports_portable_intent(policy) {
        return Err(BackendFailureKind::Unsupported);
    }
    let (attachment, dns) = attachment_and_dns(policy)?;
    let mut reservations = Vec::with_capacity(policy.published_ports().len());
    let mut mac_publications = Vec::with_capacity(policy.published_ports().len());
    let mut effective = Vec::with_capacity(policy.published_ports().len());
    for publication in policy.published_ports() {
        let HostBind::Ipv4 { address } = publication.bind() else {
            return Err(BackendFailureKind::Unsupported);
        };
        if publication.guest_port().get() < 2
            || publication
                .host_port()
                .requested()
                .is_some_and(|port| port.get() < 2)
        {
            return Err(BackendFailureKind::Unsupported);
        }
        let (reservation, host_port) =
            PortReservation::bind(address, publication.host_port(), publication.protocol())?;
        let mac_protocol = match publication.protocol() {
            TransportProtocol::Tcp => MacProtocol::Tcp,
            TransportProtocol::Udp => MacProtocol::Udp,
        };
        let mac_publication = MacPublishedPort::new(
            address,
            host_port,
            publication.guest_port().get(),
            mac_protocol,
        )
        .map_err(|_| BackendFailureKind::Unsupported)?;
        let effective_publication = EffectivePortPublication::new(
            publication.bind(),
            host_port,
            publication.guest_port().get(),
            publication.protocol(),
        )
        .map_err(|_| BackendFailureKind::IsolationFailure)?;
        reservations.push(reservation);
        mac_publications.push(mac_publication);
        effective.push(effective_publication);
    }
    let configuration = NetworkConfiguration::new(attachment, dns, mac_publications)
        .map_err(|_| BackendFailureKind::IsolationFailure)?;
    Ok(PreparedNetwork {
        configuration,
        reservations,
        publications: effective,
    })
}

fn supports_portable_intent(policy: &NetworkPolicy) -> bool {
    if !policy.proxy().is_disabled() {
        return false;
    }
    if policy.profile().is_disabled() {
        return policy.guest_addresses().all_disabled()
            && policy.egress() == EgressPolicy::Denied
            && policy.dns() == &DnsPolicy::Denied
            && policy.published_ports().is_empty();
    }
    if !policy.profile().is_operator_default() {
        return false;
    }
    if policy.guest_addresses().is_runtime_default() {
        return policy.egress() == EgressPolicy::Unspecified
            && policy.dns() == &DnsPolicy::Unspecified
            && policy.published_ports().is_empty();
    }
    let addresses = policy.guest_addresses();
    addresses.ipv4().is_enabled()
        && addresses.ipv4().requested_address().is_none()
        && addresses.ipv6().is_disabled()
}

fn attachment_and_dns(
    policy: &NetworkPolicy,
) -> Result<(MacAttachment, DnsConfiguration), BackendFailureKind> {
    match (
        policy.egress(),
        policy.dns(),
        policy.published_ports().is_empty(),
    ) {
        (EgressPolicy::Unspecified, DnsPolicy::Unspecified, true) => {
            Ok((MacAttachment::Unspecified, DnsConfiguration::RuntimeDefault))
        }
        (EgressPolicy::Denied, DnsPolicy::Denied, true) => {
            Ok((MacAttachment::Denied, DnsConfiguration::RuntimeDefault))
        }
        (EgressPolicy::Unrestricted, DnsPolicy::Unspecified, _) => {
            Ok((MacAttachment::Allowed, DnsConfiguration::RuntimeDefault))
        }
        (EgressPolicy::Unrestricted, DnsPolicy::Custom { servers }, _) => Ok((
            MacAttachment::Allowed,
            DnsConfiguration::custom(servers.clone())
                .map_err(|_| BackendFailureKind::IsolationFailure)?,
        )),
        _ => Err(BackendFailureKind::Unsupported),
    }
}
