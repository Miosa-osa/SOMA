//! Typed broker intent admitted from the portable request against one operator profile.
//!
//! Admission fails closed: an unspecified egress or DNS dimension, a proxy profile, a static
//! or IPv6 guest address, a resolver inside the protected set, or a named profile this broker
//! does not serve is rejected before any kernel object exists.

use std::net::{IpAddr, Ipv4Addr};

use sha2::{Digest, Sha256};
use soma::{DnsPolicy, EgressPolicy, NetworkPolicy, PortPublication};

use crate::{Error, IntentRejection, NetworkProfile, ProfileDigest};

mod codec;

pub use codec::MAX_ENCODED_INTENT;

/// The admitted egress class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EgressClass {
    /// No guest-originated egress beyond the gateway probe.
    Denied,
    /// Public destinations only; the protected set is dropped first.
    PublicInternet,
    /// Operator-admitted broader egress; the protected set is still dropped first.
    Unrestricted,
}

impl EgressClass {
    /// Returns whether forwarding rules accept any guest egress.
    #[must_use]
    pub const fn forwards(self) -> bool {
        !matches!(self, Self::Denied)
    }

    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Denied => 1,
            Self::PublicInternet => 2,
            Self::Unrestricted => 3,
        }
    }

    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Denied),
            2 => Some(Self::PublicInternet),
            3 => Some(Self::Unrestricted),
            _ => None,
        }
    }
}

/// The 32-byte digest of one admitted intent.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IntentDigest(pub [u8; 32]);

/// One admitted network intent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkIntent {
    egress: EgressClass,
    resolvers: Vec<Ipv4Addr>,
    publications: Vec<PortPublication>,
    profile: ProfileDigest,
}

impl NetworkIntent {
    /// Admits one portable policy against the served profile.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidIntent`] naming the first rejected dimension.
    pub fn admit(policy: &NetworkPolicy, profile: &NetworkProfile) -> Result<Self, Error> {
        use IntentRejection as R;
        let reject = |reason| Err(Error::InvalidIntent(reason));
        if !policy.profile().is_disabled() && !policy.profile().is_operator_default() {
            return reject(R::ProfileMismatch);
        }
        if !policy.proxy().is_disabled() {
            return reject(R::ProxyUnimplemented);
        }
        let addresses = policy.guest_addresses();
        if addresses.ipv4().requested_address().is_some() {
            return reject(R::StaticAddress);
        }
        if addresses.ipv6().is_enabled() {
            return reject(R::Ipv6Unimplemented);
        }
        let egress = match policy.egress() {
            EgressPolicy::Unspecified => return reject(R::EgressUnspecified),
            EgressPolicy::Denied => EgressClass::Denied,
            EgressPolicy::PublicInternet => EgressClass::PublicInternet,
            EgressPolicy::Unrestricted => EgressClass::Unrestricted,
        };
        let resolvers = match policy.dns() {
            DnsPolicy::Unspecified => return reject(R::DnsUnspecified),
            DnsPolicy::Denied => Vec::new(),
            DnsPolicy::System => profile.resolvers().to_vec(),
            DnsPolicy::Custom { servers } => custom_resolvers(servers, profile)?,
        };
        if egress == EgressClass::Denied && !resolvers.is_empty() {
            return reject(R::DnsWithoutEgress);
        }
        Self::new(
            egress,
            resolvers,
            policy.published_ports().to_vec(),
            profile.digest(),
        )
    }

    /// Builds one intent from already admitted parts.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidIntent`] when the publication count exceeds the portable bound.
    pub fn new(
        egress: EgressClass,
        resolvers: Vec<Ipv4Addr>,
        publications: Vec<PortPublication>,
        profile: ProfileDigest,
    ) -> Result<Self, Error> {
        if publications.len() > soma::MAX_PORT_PUBLICATIONS
            || resolvers.len() > soma::MAX_DNS_SERVERS
        {
            return Err(Error::Protocol("intent bounds"));
        }
        Ok(Self {
            egress,
            resolvers,
            publications,
            profile,
        })
    }

    /// Returns the fully denied intent for one profile.
    #[must_use]
    pub fn denied(profile: &NetworkProfile) -> Self {
        Self {
            egress: EgressClass::Denied,
            resolvers: Vec::new(),
            publications: Vec::new(),
            profile: profile.digest(),
        }
    }

    /// Returns the egress class.
    #[must_use]
    pub const fn egress(&self) -> EgressClass {
        self.egress
    }

    /// Returns the declared resolvers; empty means DNS is denied.
    #[must_use]
    pub fn resolvers(&self) -> &[Ipv4Addr] {
        &self.resolvers
    }

    /// Returns whether DNS transport is permitted to the declared resolvers.
    #[must_use]
    pub const fn dns_allowed(&self) -> bool {
        !self.resolvers.is_empty()
    }

    /// Returns the requested port publications.
    #[must_use]
    pub fn publications(&self) -> &[PortPublication] {
        &self.publications
    }

    /// Returns the digest of the profile the intent was admitted against.
    #[must_use]
    pub const fn profile(&self) -> ProfileDigest {
        self.profile
    }

    /// Computes the intent digest over the canonical encoding.
    #[must_use]
    pub fn digest(&self) -> IntentDigest {
        let mut bytes = b"soma-netd-intent-v1\0".to_vec();
        bytes.extend_from_slice(&self.encode());
        IntentDigest(Sha256::digest(&bytes).into())
    }
}

fn custom_resolvers(servers: &[IpAddr], profile: &NetworkProfile) -> Result<Vec<Ipv4Addr>, Error> {
    let mut resolvers = Vec::with_capacity(servers.len());
    for server in servers {
        let IpAddr::V4(address) = server else {
            return Err(Error::InvalidIntent(IntentRejection::ResolverFamily));
        };
        if profile.protected().contains(*server) {
            return Err(Error::InvalidIntent(IntentRejection::ResolverProtected));
        }
        resolvers.push(*address);
    }
    Ok(resolvers)
}

#[cfg(test)]
mod tests {
    use soma::{
        GuestAddressIntent, HostBind, HostPort, Ipv4AddressIntent, Ipv6AddressIntent,
        NetworkProfileSelector, ProxyPolicy, TransportProtocol,
    };

    use super::*;
    use crate::profile::tests::test_profile;

    fn policy(egress: EgressPolicy, dns: DnsPolicy) -> NetworkPolicy {
        NetworkPolicy::new(egress, dns, Vec::new()).expect("portable policy")
    }

    #[test]
    fn admission_maps_each_supported_class() {
        let profile = test_profile();
        let denied = NetworkIntent::admit(&NetworkPolicy::isolated(), &profile).expect("denied");
        assert_eq!(denied.egress(), EgressClass::Denied);
        assert!(!denied.dns_allowed());
        assert_eq!(denied, NetworkIntent::denied(&profile));
        let system = NetworkIntent::admit(
            &policy(EgressPolicy::PublicInternet, DnsPolicy::System),
            &profile,
        )
        .expect("system dns");
        assert_eq!(system.resolvers(), profile.resolvers());
        let custom = DnsPolicy::custom(vec![IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))]).expect("dns");
        let unrestricted =
            NetworkIntent::admit(&policy(EgressPolicy::Unrestricted, custom), &profile)
                .expect("custom dns");
        assert_eq!(unrestricted.resolvers(), &[Ipv4Addr::new(9, 9, 9, 9)]);
        assert_ne!(unrestricted.digest(), system.digest());
        assert_eq!(unrestricted.digest(), unrestricted.clone().digest());
    }

    #[test]
    fn admission_fails_closed_on_every_unsupported_dimension() {
        let profile = test_profile();
        let cases = [
            (
                NetworkPolicy::runtime_default(),
                IntentRejection::EgressUnspecified,
            ),
            (
                policy(
                    EgressPolicy::PublicInternet,
                    DnsPolicy::custom(vec![IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))])
                        .expect("dns"),
                ),
                IntentRejection::ResolverProtected,
            ),
            (
                NetworkPolicy::from_intent(
                    NetworkProfileSelector::operator_default(),
                    GuestAddressIntent::new(
                        Ipv4AddressIntent::allocated(),
                        Ipv6AddressIntent::allocated(),
                    )
                    .expect("addresses"),
                    ProxyPolicy::disabled(),
                    EgressPolicy::PublicInternet,
                    DnsPolicy::custom(vec!["2606:4700::1111".parse().expect("literal")])
                        .expect("dns"),
                    Vec::new(),
                )
                .expect("portable policy"),
                IntentRejection::Ipv6Unimplemented,
            ),
        ];
        for (policy, expected) in cases {
            assert_eq!(
                NetworkIntent::admit(&policy, &profile).expect_err("rejected"),
                Error::InvalidIntent(expected)
            );
        }
    }

    #[test]
    fn publications_participate_in_the_digest() {
        let profile = test_profile();
        let publication = PortPublication::new(
            HostBind::loopback_v4(),
            HostPort::from_u16(8080),
            80,
            TransportProtocol::Tcp,
        )
        .expect("publication");
        let with = NetworkPolicy::new(
            EgressPolicy::PublicInternet,
            DnsPolicy::Denied,
            vec![publication],
        )
        .expect("policy");
        let without = policy(EgressPolicy::PublicInternet, DnsPolicy::Denied);
        let with = NetworkIntent::admit(&with, &profile).expect("admitted");
        let without = NetworkIntent::admit(&without, &profile).expect("admitted");
        assert_eq!(with.publications().len(), 1);
        assert_ne!(with.digest(), without.digest());
    }
}
