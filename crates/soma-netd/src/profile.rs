//! The operator-owned network profile and its content digest.

use std::net::{IpAddr, Ipv4Addr};

use sha2::{Digest, Sha256};

use crate::{Cidr, Error, ProtectedReason, ProtectedSet, SubnetPlan};

const MAX_RESOLVERS: usize = soma::MAX_DNS_SERVERS;
const MAX_INTERFACE_NAME: usize = 15;

/// A validated Linux interface name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceName(String);

impl InterfaceName {
    /// Validates one interface name of at most 15 ASCII letters, digits, dots, or dashes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidProfile`] for an empty, long, or non-ASCII name.
    pub fn new(name: &str) -> Result<Self, Error> {
        let valid = !name.is_empty()
            && name.len() <= MAX_INTERFACE_NAME
            && name != "."
            && name != ".."
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.');
        valid
            .then(|| Self(name.to_owned()))
            .ok_or(Error::InvalidProfile("interface name"))
    }

    /// Returns the name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The 32-byte content digest of one profile.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProfileDigest(pub [u8; 32]);

/// The operator-owned network profile served by one broker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkProfile {
    uplink: InterfaceName,
    leases: SubnetPlan,
    transit: SubnetPlan,
    resolvers: Vec<Ipv4Addr>,
    protected: ProtectedSet,
}

impl NetworkProfile {
    /// Validates one profile.
    ///
    /// The lease and transit plans must not overlap, every plan is added to the protected
    /// set as a peer range, every host address is protected, and every system resolver must
    /// stay outside the protected set because DNS follows the same destination policy.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidProfile`] naming the first violated rule.
    pub fn new(
        uplink: InterfaceName,
        leases: SubnetPlan,
        transit: SubnetPlan,
        resolvers: Vec<Ipv4Addr>,
        host_addresses: &[IpAddr],
        control_plane: &[Cidr],
    ) -> Result<Self, Error> {
        if leases.cidr().overlaps(transit.cidr()) {
            return Err(Error::InvalidProfile("plans overlap"));
        }
        if resolvers.len() > MAX_RESOLVERS {
            return Err(Error::InvalidProfile("too many resolvers"));
        }
        let mut protected = ProtectedSet::certified_default();
        protected.push(leases.cidr(), ProtectedReason::Peer);
        protected.push(transit.cidr(), ProtectedReason::Peer);
        for address in host_addresses {
            protected.push(Cidr::host(*address), ProtectedReason::HostAddress);
        }
        for cidr in control_plane {
            protected.push(*cidr, ProtectedReason::ControlPlane);
        }
        for resolver in &resolvers {
            if protected.contains(IpAddr::V4(*resolver)) {
                return Err(Error::InvalidProfile("resolver protected"));
            }
        }
        Ok(Self {
            uplink,
            leases,
            transit,
            resolvers,
            protected,
        })
    }

    /// Returns the host uplink interface.
    #[must_use]
    pub const fn uplink(&self) -> &InterfaceName {
        &self.uplink
    }

    /// Returns the guest lease plan.
    #[must_use]
    pub const fn leases(&self) -> &SubnetPlan {
        &self.leases
    }

    /// Returns the veth transit plan.
    #[must_use]
    pub const fn transit(&self) -> &SubnetPlan {
        &self.transit
    }

    /// Returns the system resolvers.
    #[must_use]
    pub fn resolvers(&self) -> &[Ipv4Addr] {
        &self.resolvers
    }

    /// Returns the complete protected set.
    #[must_use]
    pub const fn protected(&self) -> &ProtectedSet {
        &self.protected
    }

    /// Computes the content digest.
    #[must_use]
    pub fn digest(&self) -> ProfileDigest {
        let mut bytes = b"soma-netd-profile-v1\0".to_vec();
        bytes.extend_from_slice(self.uplink.0.as_bytes());
        bytes.push(0);
        self.leases.cidr().encode(&mut bytes);
        self.transit.cidr().encode(&mut bytes);
        bytes.push(u8::try_from(self.resolvers.len()).unwrap_or(u8::MAX));
        for resolver in &self.resolvers {
            bytes.extend_from_slice(&resolver.octets());
        }
        self.protected.encode(&mut bytes);
        ProfileDigest(Sha256::digest(&bytes).into())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A profile over documentation ranges used by unit tests.
    pub(crate) fn test_profile() -> NetworkProfile {
        NetworkProfile::new(
            InterfaceName::new("uplink0").expect("name"),
            SubnetPlan::new(Ipv4Addr::new(10, 200, 0, 0), 16).expect("plan"),
            SubnetPlan::new(Ipv4Addr::new(10, 201, 0, 0), 16).expect("plan"),
            vec![Ipv4Addr::new(1, 1, 1, 1)],
            &[IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1))],
            &[Cidr::v4(Ipv4Addr::new(198, 51, 100, 0), 24).expect("cidr")],
        )
        .expect("profile")
    }

    #[test]
    fn profile_validation_and_digest_are_deterministic() {
        let profile = test_profile();
        assert_eq!(profile.digest(), test_profile().digest());
        assert!(
            profile
                .protected()
                .contains("203.0.113.1".parse().expect("literal"))
        );
        assert!(
            profile
                .protected()
                .contains("198.51.100.7".parse().expect("literal"))
        );
        assert!(
            profile
                .protected()
                .contains("10.200.0.2".parse().expect("literal"))
        );
        assert!(
            !profile
                .protected()
                .contains("1.1.1.1".parse().expect("literal"))
        );
        let mut other = profile.clone();
        other.resolvers.push(Ipv4Addr::new(9, 9, 9, 9));
        assert_ne!(other.digest(), profile.digest());
    }

    #[test]
    fn profile_rejects_overlap_protected_resolver_and_bad_names() {
        let plan = SubnetPlan::new(Ipv4Addr::new(10, 200, 0, 0), 16).expect("plan");
        let name = InterfaceName::new("eth0").expect("name");
        assert_eq!(
            NetworkProfile::new(name.clone(), plan.clone(), plan.clone(), vec![], &[], &[])
                .expect_err("overlap"),
            Error::InvalidProfile("plans overlap")
        );
        let transit = SubnetPlan::new(Ipv4Addr::new(10, 201, 0, 0), 16).expect("plan");
        assert_eq!(
            NetworkProfile::new(
                name,
                plan,
                transit,
                vec![Ipv4Addr::new(169, 254, 169, 254)],
                &[],
                &[]
            )
            .expect_err("protected resolver"),
            Error::InvalidProfile("resolver protected")
        );
        for bad in ["", "a-name-that-is-too-long", "bad name", "..", "ü"] {
            assert_eq!(
                InterfaceName::new(bad).expect_err("rejected"),
                Error::InvalidProfile("interface name")
            );
        }
    }
}
