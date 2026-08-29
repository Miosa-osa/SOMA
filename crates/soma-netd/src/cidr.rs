//! Exact IPv4 and IPv6 prefixes used by protected sets, plans, and rulesets.

use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

use crate::Error;

/// One validated address prefix.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub enum Cidr {
    /// An IPv4 prefix with its length.
    V4(Ipv4Addr, u8),
    /// An IPv6 prefix with its length.
    V6(Ipv6Addr, u8),
}

impl Cidr {
    /// Builds a canonical IPv4 prefix, clearing host bits.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidProfile`] for a length above 32.
    pub fn v4(address: Ipv4Addr, length: u8) -> Result<Self, Error> {
        if length > 32 {
            return Err(Error::InvalidProfile("ipv4 prefix length"));
        }
        let mask = mask4(length);
        Ok(Self::V4(
            Ipv4Addr::from_bits(address.to_bits() & mask),
            length,
        ))
    }

    /// Builds a canonical IPv6 prefix, clearing host bits.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidProfile`] for a length above 128.
    pub fn v6(address: Ipv6Addr, length: u8) -> Result<Self, Error> {
        if length > 128 {
            return Err(Error::InvalidProfile("ipv6 prefix length"));
        }
        let mask = mask6(length);
        Ok(Self::V6(
            Ipv6Addr::from_bits(address.to_bits() & mask),
            length,
        ))
    }

    /// Builds a host prefix for one address.
    #[must_use]
    pub fn host(address: IpAddr) -> Self {
        match address {
            IpAddr::V4(address) => Self::V4(address, 32),
            IpAddr::V6(address) => Self::V6(address, 128),
        }
    }

    /// Returns whether the address lies inside the prefix.
    #[must_use]
    pub fn contains(self, address: IpAddr) -> bool {
        match (self, address) {
            (Self::V4(network, length), IpAddr::V4(address)) => {
                address.to_bits() & mask4(length) == network.to_bits()
            }
            (Self::V6(network, length), IpAddr::V6(address)) => {
                address.to_bits() & mask6(length) == network.to_bits()
            }
            _ => false,
        }
    }

    /// Returns whether both prefixes share any address.
    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        self.contains(other.first()) || other.contains(self.first())
    }

    /// Returns the first address of the prefix.
    #[must_use]
    pub const fn first(self) -> IpAddr {
        match self {
            Self::V4(address, _) => IpAddr::V4(address),
            Self::V6(address, _) => IpAddr::V6(address),
        }
    }

    /// Returns whether the prefix is IPv4.
    #[must_use]
    pub const fn is_v4(self) -> bool {
        matches!(self, Self::V4(..))
    }

    /// Appends the canonical 18-byte encoding used by digests.
    pub fn encode(self, out: &mut Vec<u8>) {
        match self {
            Self::V4(address, length) => {
                out.push(4);
                out.extend_from_slice(&address.octets());
                out.extend_from_slice(&[0; 12]);
                out.push(length);
            }
            Self::V6(address, length) => {
                out.push(6);
                out.extend_from_slice(&address.octets());
                out.push(length);
            }
        }
    }
}

impl fmt::Display for Cidr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V4(address, length) => write!(formatter, "{address}/{length}"),
            Self::V6(address, length) => write!(formatter, "{address}/{length}"),
        }
    }
}

const fn mask4(length: u8) -> u32 {
    if length == 0 {
        0
    } else {
        u32::MAX << (32 - length as u32)
    }
}

const fn mask6(length: u8) -> u128 {
    if length == 0 {
        0
    } else {
        u128::MAX << (128 - length as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_canonicalize_contain_and_overlap() {
        let private = Cidr::v4(Ipv4Addr::new(10, 9, 8, 7), 8).expect("valid");
        assert_eq!(private.to_string(), "10.0.0.0/8");
        assert!(private.contains(IpAddr::V4(Ipv4Addr::new(10, 200, 0, 2))));
        assert!(!private.contains(IpAddr::V4(Ipv4Addr::new(11, 0, 0, 1))));
        assert!(!private.contains(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        let lease = Cidr::v4(Ipv4Addr::new(10, 200, 0, 0), 16).expect("valid");
        assert!(private.overlaps(lease));
        assert!(lease.overlaps(private));
        let ula = Cidr::v6("fd00:ec2::254".parse().expect("literal"), 7).expect("valid");
        assert_eq!(ula.to_string(), "fc00::/7");
        assert!(ula.contains("fd00:ec2::254".parse().expect("literal")));
        assert_eq!(Cidr::v4(Ipv4Addr::LOCALHOST, 33).expect_err("too long"), {
            Error::InvalidProfile("ipv4 prefix length")
        });
        assert_eq!(
            Cidr::v4(Ipv4Addr::LOCALHOST, 0).expect("any").to_string(),
            { "0.0.0.0/0" }
        );
        let mut bytes = Vec::new();
        Cidr::host(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))).encode(&mut bytes);
        assert_eq!(bytes.len(), 18);
        assert_eq!(bytes[..5], [4, 1, 2, 3, 4]);
        assert_eq!(bytes[17], 32);
    }
}
