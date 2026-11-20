//! The broker this host is configured to reach, and the intent one request is admitted to.
//!
//! Admission is the caller's own act: the launcher states what the request asked for, against
//! the profile the operator says this broker serves, and the broker records the digest of what
//! it was handed. So an operator who points the launcher at a broker serving a different profile
//! gets a recorded mismatch rather than a silently different network.

use std::{
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
};

use soma::{EgressPolicy, NetworkPolicy};
use soma_netd::{InterfaceName, NetworkIntent, NetworkProfile, SubnetPlan};

/// The environment variable naming the broker's control socket.
const SOCKET: &str = "SOMA_NETD_SOCKET";
/// The environment variable naming the host uplink the broker masquerades through.
const UPLINK: &str = "SOMA_NETD_UPLINK";
/// The environment variable listing the broker's system resolvers.
const RESOLVERS: &str = "SOMA_NETD_RESOLVERS";
/// The environment variable listing host addresses the broker protects.
const HOST_ADDRESSES: &str = "SOMA_NETD_HOST_ADDRESSES";

/// The guest lease plan every broker serves; it must match the broker's own.
const LEASES: (Ipv4Addr, u8) = (Ipv4Addr::new(10, 200, 0, 0), 16);
/// The veth transit plan every broker serves; it must match the broker's own.
const TRANSIT: (Ipv4Addr, u8) = (Ipv4Addr::new(10, 201, 0, 0), 16);

/// The broker this host is configured to reach.
#[derive(Clone, Debug)]
pub(crate) struct BrokerConfiguration {
    /// The control socket path.
    pub(crate) socket: PathBuf,
    /// The profile the operator says that broker serves.
    pub(crate) profile: NetworkProfile,
}

impl BrokerConfiguration {
    /// Reads the configured broker, or reports that this host has none.
    ///
    /// Absence is deliberate and total: with no socket named, no launch reaches the broker at
    /// all, and a request that needed egress is refused rather than served by a machine whose
    /// network is a placeholder.
    pub(crate) fn from_environment() -> Option<Self> {
        let socket = PathBuf::from(std::env::var_os(SOCKET)?);
        let uplink = std::env::var(UPLINK).ok()?;
        let profile = NetworkProfile::new(
            InterfaceName::new(&uplink).ok()?,
            SubnetPlan::new(LEASES.0, LEASES.1).ok()?,
            SubnetPlan::new(TRANSIT.0, TRANSIT.1).ok()?,
            addresses(RESOLVERS).into_iter().collect(),
            &addresses(HOST_ADDRESSES)
                .into_iter()
                .map(IpAddr::V4)
                .collect::<Vec<_>>(),
            &[],
        )
        .ok()?;
        Some(Self { socket, profile })
    }

    /// Admits one portable policy against the configured profile.
    pub(crate) fn admit(&self, policy: &NetworkPolicy) -> Option<NetworkIntent> {
        NetworkIntent::admit(policy, &self.profile).ok()
    }
}

/// Whether a launch can be given `policy` on a host configured with `configuration`, decided as
/// [`super::Egress::claim`] decides it for a launch that is not windowed.
///
/// No egress is the placeholder device, and an unstated preference degrades to it, so both are
/// always served. Anything else needs a configured broker whose profile admits the policy.
pub(crate) fn serves(configuration: Option<&BrokerConfiguration>, policy: &NetworkPolicy) -> bool {
    match policy.egress() {
        EgressPolicy::Denied | EgressPolicy::Unspecified => true,
        _ => configuration.is_some_and(|configuration| configuration.admit(policy).is_some()),
    }
}

/// Reads one comma-separated list of IPv4 addresses, dropping nothing silently.
///
/// An unparseable entry yields an empty list rather than a shorter one: a resolver list the
/// operator half-wrote would otherwise become a profile digest nothing else agrees with.
fn addresses(variable: &str) -> Vec<Ipv4Addr> {
    std::env::var(variable).map_or_else(|_| Vec::new(), |value| parse_addresses(&value))
}

/// Parses one comma-separated list, or nothing at all when any entry is not an address.
fn parse_addresses(value: &str) -> Vec<Ipv4Addr> {
    value
        .split(',')
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| entry.trim().parse::<Ipv4Addr>())
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use soma::{DnsPolicy, EgressPolicy};
    use soma_netd::EgressClass;

    use super::*;

    fn configuration() -> BrokerConfiguration {
        BrokerConfiguration {
            socket: PathBuf::from("/run/soma-netd/broker.sock"),
            profile: NetworkProfile::new(
                InterfaceName::new("eth0").expect("interface"),
                SubnetPlan::new(LEASES.0, LEASES.1).expect("leases"),
                SubnetPlan::new(TRANSIT.0, TRANSIT.1).expect("transit"),
                vec![Ipv4Addr::new(1, 1, 1, 1)],
                &[],
                &[],
            )
            .expect("profile"),
        }
    }

    #[test]
    fn a_request_for_internet_egress_with_system_dns_is_admitted_with_the_profile_resolvers() {
        let policy =
            NetworkPolicy::new(EgressPolicy::PublicInternet, DnsPolicy::System, Vec::new())
                .expect("policy");
        let intent = configuration().admit(&policy).expect("admitted");
        assert_eq!(intent.egress(), EgressClass::PublicInternet);
        assert_eq!(intent.resolvers(), [Ipv4Addr::new(1, 1, 1, 1)]);
    }

    /// An unstated dimension is never guessed at, because guessing would open something.
    #[test]
    fn an_unspecified_dimension_is_never_admitted() {
        let policy = NetworkPolicy::new(EgressPolicy::Unspecified, DnsPolicy::Unspecified, vec![])
            .expect("policy");
        assert!(configuration().admit(&policy).is_none());
    }

    /// A policy the operator's profile does not serve is refused rather than reshaped.
    #[test]
    fn a_statically_addressed_request_is_not_admitted() {
        let policy = NetworkPolicy::from_intent(
            soma::NetworkProfileSelector::operator_default(),
            soma::GuestAddressIntent::new(
                soma::Ipv4AddressIntent::requested(std::net::Ipv4Addr::new(10, 200, 0, 9))
                    .expect("address"),
                soma::Ipv6AddressIntent::disabled(),
            )
            .expect("addresses"),
            soma::ProxyPolicy::disabled(),
            EgressPolicy::PublicInternet,
            DnsPolicy::System,
            Vec::new(),
        )
        .expect("policy");
        assert!(configuration().admit(&policy).is_none());
    }

    /// SOMA#56: a runner whose default shape asked for the public internet on hosts with no
    /// broker failed every create with `unsupported_backend`; this is the answer that stops it
    /// at startup instead.
    #[test]
    fn egress_is_served_only_by_a_broker_that_admits_it_and_no_egress_always() {
        let internet =
            NetworkPolicy::new(EgressPolicy::PublicInternet, DnsPolicy::System, Vec::new())
                .expect("policy");
        let isolated = soma::Capabilities::isolated().network_policy().clone();
        let unstated =
            NetworkPolicy::new(EgressPolicy::Unspecified, DnsPolicy::Unspecified, vec![])
                .expect("policy");
        let broker = configuration();

        assert!(!serves(None, &internet), "no broker, no egress");
        assert!(serves(Some(&broker), &internet));
        assert!(serves(None, &isolated));
        assert!(serves(None, &unstated), "an unstated preference degrades");
    }

    #[test]
    fn a_half_written_address_list_yields_no_addresses_at_all() {
        assert!(parse_addresses("10.0.0.1,not-an-address").is_empty());
        assert_eq!(
            parse_addresses("10.0.0.1, 10.0.0.2"),
            [Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2)]
        );
        assert!(parse_addresses("").is_empty());
    }
}
