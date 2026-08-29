use std::collections::BTreeSet;

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::ValidationError;

use super::{
    DnsPolicy, EgressPolicy, GuestAddressIntent, Ipv4AddressIntent, Ipv6AddressIntent,
    NetworkProfileSelector, PortPublication, ProxyPolicy,
};

/// Maximum number of host port publications accepted in one portable request.
pub const MAX_PORT_PUBLICATIONS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NetworkPolicy {
    profile: NetworkProfileSelector,
    guest_addresses: GuestAddressIntent,
    proxy: ProxyPolicy,
    egress: EgressPolicy,
    dns: DnsPolicy,
    published_ports: Vec<PortPublication>,
}

impl NetworkPolicy {
    /// Returns the fail-closed policy used by a new Machine shape.
    #[must_use]
    pub const fn isolated() -> Self {
        Self {
            profile: NetworkProfileSelector::disabled(),
            guest_addresses: GuestAddressIntent::disabled(),
            proxy: ProxyPolicy::disabled(),
            egress: EgressPolicy::Denied,
            dns: DnsPolicy::Denied,
            published_ports: Vec::new(),
        }
    }

    /// Leaves every runtime-controlled network dimension to the operator default profile.
    #[must_use]
    pub const fn runtime_default() -> Self {
        Self {
            profile: NetworkProfileSelector::operator_default(),
            guest_addresses: GuestAddressIntent::runtime_default(),
            proxy: ProxyPolicy::disabled(),
            egress: EgressPolicy::Unspecified,
            dns: DnsPolicy::Unspecified,
            published_ports: Vec::new(),
        }
    }

    /// Creates a policy through the legacy egress, DNS, and publication interface.
    ///
    /// Connected and ingress-only policies receive an allocated IPv4 address and disabled IPv6.
    /// Unspecified egress remains the exact all-or-nothing runtime default.
    ///
    /// # Errors
    ///
    /// Applies the same full invariant gate as [`Self::from_intent`].
    pub fn new(
        egress: EgressPolicy,
        dns: DnsPolicy,
        published_ports: Vec<PortPublication>,
    ) -> Result<Self, ValidationError> {
        let guest_addresses = if egress == EgressPolicy::Unspecified {
            GuestAddressIntent::runtime_default()
        } else if egress == EgressPolicy::Denied && published_ports.is_empty() {
            GuestAddressIntent::disabled()
        } else {
            GuestAddressIntent::new(
                Ipv4AddressIntent::allocated(),
                Ipv6AddressIntent::disabled(),
            )?
        };
        let profile = if egress == EgressPolicy::Denied && published_ports.is_empty() {
            NetworkProfileSelector::disabled()
        } else {
            NetworkProfileSelector::operator_default()
        };
        Self::from_intent(
            profile,
            guest_addresses,
            ProxyPolicy::disabled(),
            egress,
            dns,
            published_ports,
        )
    }

    /// Creates one canonical portable network intent.
    ///
    /// # Errors
    ///
    /// Rejects partial runtime defaults, inconsistent address, DNS, proxy, and egress intent,
    /// oversized publication sets, and endpoint collisions before host state can change.
    pub fn from_intent(
        profile: NetworkProfileSelector,
        guest_addresses: GuestAddressIntent,
        proxy: ProxyPolicy,
        egress: EgressPolicy,
        dns: DnsPolicy,
        mut published_ports: Vec<PortPublication>,
    ) -> Result<Self, ValidationError> {
        if !valid_intent(
            &profile,
            &guest_addresses,
            &proxy,
            egress,
            &dns,
            &published_ports,
        ) {
            return Err(ValidationError::InvalidNetworkPolicy);
        }
        published_ports.sort_unstable();
        Ok(Self {
            profile,
            guest_addresses,
            proxy,
            egress,
            dns,
            published_ports,
        })
    }

    #[must_use]
    pub const fn profile(&self) -> &NetworkProfileSelector {
        &self.profile
    }

    #[must_use]
    pub const fn guest_addresses(&self) -> &GuestAddressIntent {
        &self.guest_addresses
    }

    #[must_use]
    pub const fn proxy(&self) -> &ProxyPolicy {
        &self.proxy
    }

    #[must_use]
    pub const fn egress(&self) -> EgressPolicy {
        self.egress
    }

    #[must_use]
    pub const fn dns(&self) -> &DnsPolicy {
        &self.dns
    }

    #[must_use]
    pub fn published_ports(&self) -> &[PortPublication] {
        &self.published_ports
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NetworkPolicyWire {
    profile: NetworkProfileSelector,
    guest_addresses: GuestAddressIntent,
    proxy: ProxyPolicy,
    egress: EgressPolicy,
    dns: DnsPolicy,
    published_ports: Vec<PortPublication>,
}

impl<'de> Deserialize<'de> for NetworkPolicy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = NetworkPolicyWire::deserialize(deserializer)?;
        Self::from_intent(
            wire.profile,
            wire.guest_addresses,
            wire.proxy,
            wire.egress,
            wire.dns,
            wire.published_ports,
        )
        .map_err(D::Error::custom)
    }
}

fn valid_intent(
    profile: &NetworkProfileSelector,
    addresses: &GuestAddressIntent,
    proxy: &ProxyPolicy,
    egress: EgressPolicy,
    dns: &DnsPolicy,
    publications: &[PortPublication],
) -> bool {
    if publications.len() > MAX_PORT_PUBLICATIONS || has_publication_collision(publications) {
        return false;
    }
    let has_unspecified = addresses.is_runtime_default()
        || egress == EgressPolicy::Unspecified
        || dns == &DnsPolicy::Unspecified;
    if has_unspecified {
        return profile.is_operator_default()
            && addresses.is_runtime_default()
            && egress == EgressPolicy::Unspecified
            && dns == &DnsPolicy::Unspecified
            && proxy.is_disabled()
            && publications.is_empty();
    }
    if profile.is_disabled() {
        return addresses.all_disabled()
            && proxy.is_disabled()
            && egress == EgressPolicy::Denied
            && dns == &DnsPolicy::Denied
            && publications.is_empty();
    }
    if !publications.is_empty() && !addresses.any_enabled() {
        return false;
    }
    if !valid_dns(addresses, dns) {
        return false;
    }
    match egress {
        EgressPolicy::Unspecified => false,
        EgressPolicy::Denied => dns == &DnsPolicy::Denied && proxy.is_disabled(),
        EgressPolicy::PublicInternet => addresses.any_enabled(),
        EgressPolicy::Unrestricted => addresses.any_enabled() && proxy.is_disabled(),
    }
}

fn valid_dns(addresses: &GuestAddressIntent, dns: &DnsPolicy) -> bool {
    match dns {
        DnsPolicy::Unspecified => false,
        DnsPolicy::Denied => true,
        DnsPolicy::System => addresses.any_enabled(),
        DnsPolicy::Custom { servers } => {
            addresses.any_enabled()
                && servers.iter().all(|server| match server {
                    std::net::IpAddr::V4(_) => addresses.ipv4().is_enabled(),
                    std::net::IpAddr::V6(_) => addresses.ipv6().is_enabled(),
                })
        }
    }
}

fn has_publication_collision(publications: &[PortPublication]) -> bool {
    let mut complete = BTreeSet::new();
    for publication in publications {
        let identity = (
            publication.bind(),
            publication.host_port(),
            publication.guest_port(),
            publication.protocol(),
        );
        if !complete.insert(identity) {
            return true;
        }
    }
    publications.iter().enumerate().any(|(index, left)| {
        let Some(left_port) = left.host_port().requested() else {
            return false;
        };
        publications[index + 1..].iter().any(|right| {
            right.host_port().requested() == Some(left_port)
                && right.protocol() == left.protocol()
                && right.bind().conflicts(left.bind())
        })
    })
}
