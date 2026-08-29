//! The protected destination set that every egress mode drops before any accept.
//!
//! The certified default is built from the ranges named in ADR 0012 and the threat model:
//! loopback, link-local, RFC 1918 and ULA space, multicast, broadcast, the unspecified and
//! reserved ranges, and the cloud metadata endpoints documented by AWS, Google Cloud, and
//! Azure in `RESOURCES.md`.
//! Operators extend it with host addresses, peer plans, and control-plane prefixes.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::Cidr;

/// Why one prefix is protected.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[allow(missing_docs)]
pub enum ProtectedReason {
    Unspecified,
    Loopback,
    LinkLocal,
    Rfc1918,
    SharedAddressSpace,
    UniqueLocal,
    Multicast,
    Broadcast,
    Reserved,
    Benchmark,
    Ipv4Mapped,
    AwsMetadata,
    AwsResolver,
    AwsTimeSync,
    AwsEcsMetadata,
    GoogleMetadata,
    AzureMetadata,
    AzureWireServer,
    HostAddress,
    Peer,
    ControlPlane,
}

/// One protected prefix with its reason.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct ProtectedDestination {
    /// The prefix.
    pub cidr: Cidr,
    /// The reason it is protected.
    pub reason: ProtectedReason,
}

/// The complete ordered protected set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectedSet {
    entries: Vec<ProtectedDestination>,
}

const fn v4(a: u8, b: u8, c: u8, d: u8, length: u8) -> Cidr {
    Cidr::V4(Ipv4Addr::new(a, b, c, d), length)
}

const fn v6(segments: [u16; 8], length: u8) -> Cidr {
    Cidr::V6(
        Ipv6Addr::new(
            segments[0],
            segments[1],
            segments[2],
            segments[3],
            segments[4],
            segments[5],
            segments[6],
            segments[7],
        ),
        length,
    )
}

/// The certified default protected destinations, IPv4 first, in ruleset order.
pub const CERTIFIED_DEFAULT: [ProtectedDestination; 26] = {
    use ProtectedReason as R;
    [
        ProtectedDestination {
            cidr: v4(0, 0, 0, 0, 8),
            reason: R::Unspecified,
        },
        ProtectedDestination {
            cidr: v4(10, 0, 0, 0, 8),
            reason: R::Rfc1918,
        },
        ProtectedDestination {
            cidr: v4(100, 64, 0, 0, 10),
            reason: R::SharedAddressSpace,
        },
        ProtectedDestination {
            cidr: v4(127, 0, 0, 0, 8),
            reason: R::Loopback,
        },
        ProtectedDestination {
            cidr: v4(169, 254, 0, 0, 16),
            reason: R::LinkLocal,
        },
        ProtectedDestination {
            cidr: v4(169, 254, 169, 254, 32),
            reason: R::AwsMetadata,
        },
        ProtectedDestination {
            cidr: v4(169, 254, 169, 254, 32),
            reason: R::GoogleMetadata,
        },
        ProtectedDestination {
            cidr: v4(169, 254, 169, 254, 32),
            reason: R::AzureMetadata,
        },
        ProtectedDestination {
            cidr: v4(169, 254, 169, 253, 32),
            reason: R::AwsResolver,
        },
        ProtectedDestination {
            cidr: v4(169, 254, 169, 123, 32),
            reason: R::AwsTimeSync,
        },
        ProtectedDestination {
            cidr: v4(169, 254, 170, 2, 32),
            reason: R::AwsEcsMetadata,
        },
        ProtectedDestination {
            cidr: v4(168, 63, 129, 16, 32),
            reason: R::AzureWireServer,
        },
        ProtectedDestination {
            cidr: v4(172, 16, 0, 0, 12),
            reason: R::Rfc1918,
        },
        ProtectedDestination {
            cidr: v4(192, 0, 0, 0, 24),
            reason: R::Reserved,
        },
        ProtectedDestination {
            cidr: v4(192, 168, 0, 0, 16),
            reason: R::Rfc1918,
        },
        ProtectedDestination {
            cidr: v4(198, 18, 0, 0, 15),
            reason: R::Benchmark,
        },
        ProtectedDestination {
            cidr: v4(224, 0, 0, 0, 4),
            reason: R::Multicast,
        },
        ProtectedDestination {
            cidr: v4(240, 0, 0, 0, 4),
            reason: R::Reserved,
        },
        ProtectedDestination {
            cidr: v4(255, 255, 255, 255, 32),
            reason: R::Broadcast,
        },
        ProtectedDestination {
            cidr: v6([0; 8], 128),
            reason: R::Unspecified,
        },
        ProtectedDestination {
            cidr: v6([0, 0, 0, 0, 0, 0, 0, 1], 128),
            reason: R::Loopback,
        },
        ProtectedDestination {
            cidr: v6([0, 0, 0, 0, 0, 0xffff, 0, 0], 96),
            reason: R::Ipv4Mapped,
        },
        ProtectedDestination {
            cidr: v6([0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x254], 128),
            reason: R::AwsMetadata,
        },
        ProtectedDestination {
            cidr: v6([0xfc00, 0, 0, 0, 0, 0, 0, 0], 7),
            reason: R::UniqueLocal,
        },
        ProtectedDestination {
            cidr: v6([0xfe80, 0, 0, 0, 0, 0, 0, 0], 10),
            reason: R::LinkLocal,
        },
        ProtectedDestination {
            cidr: v6([0xff00, 0, 0, 0, 0, 0, 0, 0], 8),
            reason: R::Multicast,
        },
    ]
};

impl ProtectedSet {
    /// Returns the certified default set.
    #[must_use]
    pub fn certified_default() -> Self {
        Self {
            entries: CERTIFIED_DEFAULT.to_vec(),
        }
    }

    /// Appends one operator entry.
    pub fn push(&mut self, cidr: Cidr, reason: ProtectedReason) {
        self.entries.push(ProtectedDestination { cidr, reason });
    }

    /// Returns every entry in ruleset order.
    #[must_use]
    pub fn entries(&self) -> &[ProtectedDestination] {
        &self.entries
    }

    /// Returns whether the address is protected.
    #[must_use]
    pub fn contains(&self, address: IpAddr) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.cidr.contains(address))
    }

    /// Appends the canonical encoding used by profile digests.
    pub fn encode(&self, out: &mut Vec<u8>) {
        for entry in &self.entries {
            entry.cidr.encode(out);
            out.push(entry.reason as u8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("literal")
    }

    #[test]
    fn certified_default_covers_every_named_protected_class() {
        let set = ProtectedSet::certified_default();
        for text in [
            "127.0.0.1",
            "10.0.0.1",
            "172.31.255.254",
            "192.168.1.1",
            "169.254.169.254",
            "169.254.169.253",
            "169.254.169.123",
            "169.254.170.2",
            "168.63.129.16",
            "100.100.100.200",
            "224.0.0.1",
            "255.255.255.255",
            "0.0.0.0",
            "::1",
            "::",
            "fd00:ec2::254",
            "fe80::1",
            "ff02::1",
            "::ffff:169.254.169.254",
        ] {
            assert!(set.contains(ip(text)), "{text} must be protected");
        }
        for text in ["1.1.1.1", "8.8.8.8", "203.0.113.10", "2606:4700::1111"] {
            assert!(!set.contains(ip(text)), "{text} must stay public");
        }
    }

    #[test]
    fn operator_entries_extend_the_default() {
        let mut set = ProtectedSet::certified_default();
        set.push(Cidr::host(ip("203.0.113.1")), ProtectedReason::ControlPlane);
        assert!(set.contains(ip("203.0.113.1")));
        assert_eq!(set.entries().len(), CERTIFIED_DEFAULT.len() + 1);
    }
}
