//! CIDR parsing, canonical rendering, and containment.
//!
//! A CIDR is locked in exactly one text: the network address with every host bit clear as
//! the standard library renders it (RFC 5952 for IPv6), a `/`, and the prefix length without
//! leading zeros.
//! Authored text with host bits set is rejected rather than silently masked, because two
//! enforcement tools may read such a value as different networks.

use std::net::IpAddr;

use crate::rejection::InvalidReason;

/// One IPv4 or IPv6 network with every host bit clear.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Cidr {
    address: IpAddr,
    prefix: u8,
}

impl Cidr {
    /// Parses `address/prefix`, rejecting a malformed address, a prefix with leading zeros
    /// or beyond the family width, and any set host bit.
    pub(crate) fn parse(value: &str) -> Result<Self, InvalidReason> {
        let (address, prefix) = value.split_once('/').ok_or(InvalidReason::InvalidCidr)?;
        let address: IpAddr = address.parse().map_err(|_| InvalidReason::InvalidCidr)?;
        let well_formed = !prefix.is_empty()
            && prefix.len() <= 3
            && prefix.bytes().all(|byte| byte.is_ascii_digit())
            && (prefix.len() == 1 || !prefix.starts_with('0'));
        if !well_formed {
            return Err(InvalidReason::InvalidCidr);
        }
        let prefix: u8 = prefix.parse().map_err(|_| InvalidReason::InvalidCidr)?;
        let width = if address.is_ipv4() { 32 } else { 128 };
        if prefix > width {
            return Err(InvalidReason::InvalidCidr);
        }
        let cidr = Self { address, prefix };
        if numeric(address) & !cidr.mask() != 0 {
            return Err(InvalidReason::InvalidCidr);
        }
        Ok(cidr)
    }

    /// The one canonical text of this network.
    pub(crate) fn canonical(&self) -> String {
        format!("{}/{}", self.address, self.prefix)
    }

    /// Whether `self` contains every address of `other`.
    pub(crate) fn contains(&self, other: &Self) -> bool {
        self.address.is_ipv4() == other.address.is_ipv4()
            && self.prefix <= other.prefix
            && numeric(self.address) & self.mask() == numeric(other.address) & self.mask()
    }

    /// Both families are left-aligned in 128 bits by `numeric`, so the mask always starts at
    /// the top bit regardless of family.
    fn mask(&self) -> u128 {
        if self.prefix == 0 {
            0
        } else {
            u128::MAX << (128 - u32::from(self.prefix))
        }
    }
}

/// Whether `value` is already the canonical text of a valid network.
pub(crate) fn is_canonical(value: &str) -> bool {
    Cidr::parse(value).is_ok_and(|cidr| cidr.canonical() == value)
}

/// The sorted, deduplicated canonical texts of `values`; text that does not parse is kept
/// verbatim so a later shape check reports it.
pub(crate) fn canonical_list(values: &[String]) -> Vec<String> {
    let mut canonical: Vec<String> = values
        .iter()
        .map(|value| Cidr::parse(value).map_or_else(|_| value.clone(), |cidr| cidr.canonical()))
        .collect();
    canonical.sort();
    canonical.dedup();
    canonical
}

fn numeric(address: IpAddr) -> u128 {
    match address {
        IpAddr::V4(v4) => u128::from(u32::from(v4)) << 96,
        IpAddr::V6(v6) => u128::from(v6),
    }
}
