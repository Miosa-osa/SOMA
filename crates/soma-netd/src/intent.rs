//! Typed broker intent admitted from the portable request against one operator profile.
//!
//! Admission fails closed: an unspecified egress or DNS dimension, a proxy profile, a static
//! or IPv6 guest address, a resolver inside the protected set, an IPv6 host bind on a
//! publication, or a named profile this broker does not serve is rejected before any kernel
//! object exists.

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
        // A publication is translated onto the guest's IPv4 lease, so an IPv6 host bind has no
        // destination to name in this profile slice. It is refused here with every other
        // unsupported dimension rather than at the kernel, where the Instance would already be
        // running and the caller would learn only that activation failed.
        if policy
            .published_ports()
            .iter()
            .any(|publication| publication.bind().address().is_ipv6())
        {
            return reject(R::PublicationFamily);
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
mod tests;
