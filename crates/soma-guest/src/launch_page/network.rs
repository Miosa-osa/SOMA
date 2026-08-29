use crate::Error;

use super::wire::Reader;

/// Encoded byte size of the non-secret network fields inside the launch page.
pub(super) const ENCODED_SIZE: usize = 4 + 4 + 6 + 4 + 1 + 4 + 4 + 8;

const VMADDR_CID_RESERVED_MAX: u32 = 2;
const MAX_PREFIX_LENGTH: u8 = 30;

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
    /// Validates one fresh network identity.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidLaunchNetwork`] for a reserved vsock CID, a zero generation, a
    /// multicast or zero MAC, an unusable IPv4 address, an invalid prefix, a gateway outside the
    /// prefix or equal to the address, a zero resolver, or a zero time sample.
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
        let valid = self.vsock_cid > VMADDR_CID_RESERVED_MAX
            && self.vsock_cid != u32::MAX
            && self.generation != 0
            && self.mac != [0; 6]
            && self.mac[0] & 1 == 0
            && usable_unicast(self.address)
            && usable_unicast(self.gateway)
            && usable_unicast(self.resolver)
            && (1..=MAX_PREFIX_LENGTH).contains(&self.prefix_length)
            && address & mask == gateway & mask
            && address != gateway
            && self.time_sample_nanos != 0;
        valid.then_some(()).ok_or(Error::InvalidLaunchNetwork)
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

const fn usable_unicast(address: [u8; 4]) -> bool {
    let first = address[0];
    let value = u32::from_be_bytes(address);
    value != 0 && value != u32::MAX && first != 0 && first != 127 && first < 224
}

fn write(destination: &mut [u8], cursor: &mut usize, source: &[u8]) {
    let end = *cursor + source.len();
    destination[*cursor..end].copy_from_slice(source);
    *cursor = end;
}

#[cfg(test)]
mod tests {
    use super::*;

    type Fields = (u32, u32, [u8; 6], [u8; 4], u8, [u8; 4], [u8; 4], u64);

    fn valid() -> LaunchNetwork {
        LaunchNetwork::new(
            3,
            1,
            [0x02, 0, 0, 0, 0, 1],
            [10, 0, 0, 2],
            24,
            [10, 0, 0, 1],
            [10, 0, 0, 1],
            1,
        )
        .expect("valid network")
    }

    #[test]
    fn netmask_follows_prefix_length() {
        assert_eq!(valid().netmask(), [255, 255, 255, 0]);
        assert_eq!(prefix_mask(30), 0xFFFF_FFFC);
        assert_eq!(prefix_mask(1), 0x8000_0000);
    }

    #[test]
    fn round_trips_through_the_fixed_encoding() {
        let mut encoded = [0; ENCODED_SIZE];
        valid().encode(&mut encoded);

        assert_eq!(LaunchNetwork::decode(&encoded).expect("decodes"), valid());
        assert_eq!(
            LaunchNetwork::decode(&encoded[..ENCODED_SIZE - 1]).expect_err("short input"),
            Error::LaunchPageRejected
        );
    }

    #[test]
    fn rejects_every_invalid_field_class() {
        let base = valid();
        let mac = base.mac;
        let address = base.address;
        let gateway = base.gateway;
        let resolver = base.resolver;
        let cases: [Fields; 12] = [
            (2, 1, mac, address, 24, gateway, resolver, 1),
            (u32::MAX, 1, mac, address, 24, gateway, resolver, 1),
            (3, 0, mac, address, 24, gateway, resolver, 1),
            (3, 1, [1, 0, 0, 0, 0, 1], address, 24, gateway, resolver, 1),
            (3, 1, [0; 6], address, 24, gateway, resolver, 1),
            (3, 1, mac, [127, 0, 0, 1], 24, gateway, resolver, 1),
            (3, 1, mac, address, 0, gateway, resolver, 1),
            (3, 1, mac, address, 31, gateway, resolver, 1),
            (3, 1, mac, address, 24, [10, 0, 1, 1], resolver, 1),
            (3, 1, mac, address, 24, address, resolver, 1),
            (3, 1, mac, address, 24, gateway, [0; 4], 1),
            (3, 1, mac, address, 24, gateway, resolver, 0),
        ];
        for (cid, generation, mac, address, prefix, gateway, resolver, time) in cases {
            assert_eq!(
                LaunchNetwork::new(
                    cid, generation, mac, address, prefix, gateway, resolver, time
                )
                .expect_err("invalid network"),
                Error::InvalidLaunchNetwork
            );
        }
    }
}
