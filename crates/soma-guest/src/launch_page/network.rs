//! The non-secret IPv4 and transport identity of one launch page.
//!
//! # Declared IPv4 profile
//!
//! Every Instance receives one broadcast-capable subnet with at least two usable host
//! addresses, so the accepted prefix lengths are 1 through 30.
//! A `/31` point-to-point link is deliberately not accepted: the profile requires a distinct
//! gateway and a directed broadcast address, and RFC 3021 links have neither, so accepting one
//! would make the network and broadcast rejections below unenforceable.
//! A `/32` host route is deliberately not accepted for the same reason.
//! Widening the profile to point-to-point links is a launch-page schema decision, not a
//! validation relaxation.
//!
//! The guest address and the gateway must both be usable unicast hosts inside that subnet and
//! must differ from each other, from the subnet network address, and from its directed
//! broadcast address.
//! The resolver must be a usable unicast address; it may sit outside the subnet, because a
//! resolver reached through the gateway is a normal deployment, but when it does sit inside
//! the subnet it is held to the same host rules.
//! Usable unicast excludes the unspecified address, `0.0.0.0/8`, loopback, link-local
//! `169.254.0.0/16`, multicast and every reserved address from `224.0.0.0` up, and the limited
//! broadcast address.

use crate::Error;

use super::wire::Reader;

#[cfg(test)]
mod tests;

/// Encoded byte size of the non-secret network fields inside the launch page.
pub(super) const ENCODED_SIZE: usize = 4 + 4 + 6 + 4 + 1 + 4 + 4 + 8;

const VMADDR_CID_RESERVED_MAX: u32 = 2;
const MIN_PREFIX_LENGTH: u8 = 1;
/// The longest accepted prefix; see the declared IPv4 profile above for why `/31` and `/32`
/// are excluded.
const MAX_PREFIX_LENGTH: u8 = 30;
const LINK_LOCAL: [u8; 2] = [169, 254];
const FIRST_MULTICAST_OCTET: u8 = 224;
const LOOPBACK_OCTET: u8 = 127;

/// Non-secret fresh network and transport identity delivered with one launch page.
///
/// Every field is chosen by the VMM for one concrete Instance and must be replaced on each
/// restore so a captured identity never reaches the network.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LaunchNetwork {
    vsock_cid: u32,
    generation: u32,
    mac: [u8; 6],
    address: [u8; 4],
    prefix_length: u8,
    gateway: [u8; 4],
    resolver: [u8; 4],
    time_sample_nanos: u64,
}

impl LaunchNetwork {
    /// Validates one fresh network identity against the declared IPv4 profile.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidLaunchNetwork`] for a reserved vsock CID, a zero generation, a
    /// multicast or zero MAC, an unspecified, loopback, link-local, multicast, reserved, or
    /// broadcast IPv4 value, a prefix outside 1 through 30, a gateway outside the prefix or
    /// equal to the address, a guest or gateway that is the subnet network or directed
    /// broadcast address, an in-prefix resolver that is one of those two, or a zero time
    /// sample.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        vsock_cid: u32,
        generation: u32,
        mac: [u8; 6],
        address: [u8; 4],
        prefix_length: u8,
        gateway: [u8; 4],
        resolver: [u8; 4],
        time_sample_nanos: u64,
    ) -> Result<Self, Error> {
        let network = Self {
            vsock_cid,
            generation,
            mac,
            address,
            prefix_length,
            gateway,
            resolver,
            time_sample_nanos,
        };
        network.validate()?;
        Ok(network)
    }

    /// Returns the guest vsock context identifier assigned for this Instance.
    #[must_use]
    pub const fn vsock_cid(self) -> u32 {
        self.vsock_cid
    }

    /// Returns the network identity generation that invalidates captured network state.
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }

    /// Returns the effective unicast MAC address.
    #[must_use]
    pub const fn mac(self) -> [u8; 6] {
        self.mac
    }

    /// Returns the IPv4 address in network byte order.
    #[must_use]
    pub const fn address(self) -> [u8; 4] {
        self.address
    }

    /// Returns the IPv4 prefix length.
    #[must_use]
    pub const fn prefix_length(self) -> u8 {
        self.prefix_length
    }

    /// Returns the default gateway IPv4 address.
    #[must_use]
    pub const fn gateway(self) -> [u8; 4] {
        self.gateway
    }

    /// Returns the single resolver IPv4 address.
    #[must_use]
    pub const fn resolver(self) -> [u8; 4] {
        self.resolver
    }

    /// Returns the host wall-clock sample as Unix nanoseconds.
    #[must_use]
    pub const fn time_sample_nanos(self) -> u64 {
        self.time_sample_nanos
    }

    /// Returns the IPv4 netmask for the prefix in network byte order.
    #[must_use]
    pub const fn netmask(self) -> [u8; 4] {
        prefix_mask(self.prefix_length).to_be_bytes()
    }

    pub(super) fn encode(self, destination: &mut [u8]) {
        let mut cursor = 0;
        write(destination, &mut cursor, &self.vsock_cid.to_be_bytes());
        write(destination, &mut cursor, &self.generation.to_be_bytes());
        write(destination, &mut cursor, &self.mac);
        write(destination, &mut cursor, &self.address);
        write(destination, &mut cursor, &[self.prefix_length]);
        write(destination, &mut cursor, &self.gateway);
        write(destination, &mut cursor, &self.resolver);
        write(
            destination,
            &mut cursor,
            &self.time_sample_nanos.to_be_bytes(),
        );
        debug_assert_eq!(cursor, ENCODED_SIZE);
    }

    pub(super) fn decode(source: &[u8]) -> Result<Self, Error> {
        let mut reader = Reader::new(source);
        let network = Self::new(
            reader.u32()?,
            reader.u32()?,
            reader.array()?,
            reader.array()?,
            reader.u8()?,
            reader.array()?,
            reader.array()?,
            reader.u64()?,
        )
        .map_err(|_| Error::LaunchPageRejected)?;
        reader.finish()?;
        Ok(network)
    }

    fn validate(self) -> Result<(), Error> {
        let mask = prefix_mask(self.prefix_length);
        let address = u32::from_be_bytes(self.address);
        let gateway = u32::from_be_bytes(self.gateway);
        let resolver = u32::from_be_bytes(self.resolver);
        let subnet = Subnet {
            network: address & mask,
            broadcast: (address & mask) | !mask,
        };
        let valid = self.vsock_cid > VMADDR_CID_RESERVED_MAX
            && self.vsock_cid != u32::MAX
            && self.generation != 0
            && self.mac != [0; 6]
            && self.mac[0] & 1 == 0
            && usable_unicast(self.address)
            && usable_unicast(self.gateway)
            && usable_unicast(self.resolver)
            && (MIN_PREFIX_LENGTH..=MAX_PREFIX_LENGTH).contains(&self.prefix_length)
            && gateway & mask == subnet.network
            && address != gateway
            && subnet.usable_host(address)
            && subnet.usable_host(gateway)
            && (resolver & mask != subnet.network || subnet.usable_host(resolver))
            && self.time_sample_nanos != 0;
        valid.then_some(()).ok_or(Error::InvalidLaunchNetwork)
    }
}

/// The subnet addresses that are reserved rather than assignable to a host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Subnet {
    network: u32,
    broadcast: u32,
}

impl Subnet {
    /// Returns whether `value` is assignable rather than the network or broadcast address.
    const fn usable_host(self, value: u32) -> bool {
        value != self.network && value != self.broadcast
    }
}

const fn prefix_mask(prefix_length: u8) -> u32 {
    if prefix_length == 0 {
        0
    } else if prefix_length >= 32 {
        u32::MAX
    } else {
        u32::MAX << (32 - prefix_length)
    }
}

/// Returns whether an address is a globally usable unicast value on any subnet.
///
/// This rejects the classes that can never name a peer on the guest link: the unspecified
/// address and the rest of `0.0.0.0/8`, loopback, link-local `169.254.0.0/16`, multicast and
/// every reserved address from `224.0.0.0` up, and the limited broadcast address.
const fn usable_unicast(address: [u8; 4]) -> bool {
    let first = address[0];
    let value = u32::from_be_bytes(address);
    value != 0
        && value != u32::MAX
        && first != 0
        && first != LOOPBACK_OCTET
        && first < FIRST_MULTICAST_OCTET
        && !(first == LINK_LOCAL[0] && address[1] == LINK_LOCAL[1])
}

fn write(destination: &mut [u8], cursor: &mut usize, source: &[u8]) {
    let end = *cursor + source.len();
    destination[*cursor..end].copy_from_slice(source);
    *cursor = end;
}
