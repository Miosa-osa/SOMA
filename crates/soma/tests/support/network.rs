use std::{collections::BTreeSet, net::IpAddr};

use soma::{
    AssignedAddress, DnsPolicy, EffectiveNetwork, EffectivePortPublication, EgressPolicy, HostPort,
    NetworkAttachment, NetworkPolicy, Observation, PortActivationClass,
};

pub fn observed_network(policy: &NetworkPolicy) -> EffectiveNetwork {
    let publications = observed_publications(policy);
    let egress = match policy.egress() {
        EgressPolicy::Unspecified => EgressPolicy::Unrestricted,
        value => value,
    };
    let dns = match policy.dns() {
        DnsPolicy::Unspecified => DnsPolicy::System,
        value => value.clone(),
    };
    let attachment = if egress == EgressPolicy::Denied && publications.is_empty() {
        NetworkAttachment::Detached
    } else {
        NetworkAttachment::Attached
    };
    let activation = if publications.is_empty() {
        PortActivationClass::NotApplicable
    } else {
        PortActivationClass::AtomicSocketHandoff
    };

    EffectiveNetwork::new(
        Observation::Observed(attachment),
        Observation::Observed(egress),
        Observation::Observed(dns),
        Observation::Observed(observed_addresses(policy, attachment)),
        Observation::Observed(publications),
        Observation::Observed(activation),
    )
    .expect("test backend emits valid network evidence")
}

fn observed_addresses(
    policy: &NetworkPolicy,
    attachment: NetworkAttachment,
) -> Vec<AssignedAddress> {
    if attachment == NetworkAttachment::Detached {
        return Vec::new();
    }
    let requested = policy.guest_addresses();
    let mut addresses = Vec::new();
    if requested.ipv4().is_enabled() || requested.ipv4().is_unspecified() {
        let address = requested
            .ipv4()
            .requested_address()
            .unwrap_or(std::net::Ipv4Addr::new(192, 0, 2, 2));
        addresses
            .push(AssignedAddress::new(IpAddr::V4(address), 24).expect("valid test IPv4 address"));
    }
    if requested.ipv6().is_enabled() {
        let address = requested
            .ipv6()
            .requested_address()
            .unwrap_or(std::net::Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 2));
        addresses
            .push(AssignedAddress::new(IpAddr::V6(address), 64).expect("valid test IPv6 address"));
    }
    addresses
}

fn observed_publications(policy: &NetworkPolicy) -> Vec<EffectivePortPublication> {
    let mut used = policy
        .published_ports()
        .iter()
        .filter_map(|publication| publication.host_port().requested())
        .map(std::num::NonZeroU16::get)
        .collect::<BTreeSet<_>>();
    let mut candidate = 49_152_u16;

    policy
        .published_ports()
        .iter()
        .map(|publication| {
            let host_port = match publication.host_port() {
                HostPort::Fixed(port) => port.get(),
                HostPort::Automatic => {
                    while used.contains(&candidate) {
                        candidate = candidate.checked_add(1).expect("test port range exhausted");
                    }
                    let selected = candidate;
                    used.insert(selected);
                    candidate = candidate.checked_add(1).expect("test port range exhausted");
                    selected
                }
            };
            EffectivePortPublication::new(
                publication.bind(),
                host_port,
                publication.guest_port().get(),
                publication.protocol(),
            )
            .expect("valid observed test publication")
        })
        .collect()
}
