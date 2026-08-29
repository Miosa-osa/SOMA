//! Bounded canonical encoding of one intent for digests, the ledger, and the daemon protocol.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use soma::{HostBind, HostPort, PortPublication, TransportProtocol};

use super::{EgressClass, NetworkIntent};
use crate::{Error, ProfileDigest};

const PUBLICATION_SIZE: usize = 1 + 16 + 1 + 2 + 2 + 1;

/// The largest encoded intent: header, eight resolvers, and 32 publications.
pub const MAX_ENCODED_INTENT: usize =
    1 + 32 + 1 + 4 * soma::MAX_DNS_SERVERS + 1 + PUBLICATION_SIZE * soma::MAX_PORT_PUBLICATIONS;

impl NetworkIntent {
    /// Encodes the intent canonically.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MAX_ENCODED_INTENT);
        out.push(self.egress.code());
        out.extend_from_slice(&self.profile.0);
        out.push(u8::try_from(self.resolvers.len()).unwrap_or(u8::MAX));
        for resolver in &self.resolvers {
            out.extend_from_slice(&resolver.octets());
        }
        out.push(u8::try_from(self.publications.len()).unwrap_or(u8::MAX));
        for publication in &self.publications {
            encode_publication(publication, &mut out);
        }
        out
    }

    /// Decodes one canonical intent, rejecting trailing bytes and out-of-range values.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Protocol`] for any malformed input.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut reader = Reader { bytes, cursor: 0 };
        let egress = EgressClass::from_code(reader.u8()?).ok_or(Error::Protocol("egress"))?;
        let profile = ProfileDigest(reader.array()?);
        let resolver_count = usize::from(reader.u8()?);
        if resolver_count > soma::MAX_DNS_SERVERS {
            return Err(Error::Protocol("resolver count"));
        }
        let mut resolvers = Vec::with_capacity(resolver_count);
        for _ in 0..resolver_count {
            resolvers.push(Ipv4Addr::from(reader.array::<4>()?));
        }
        let publication_count = usize::from(reader.u8()?);
        if publication_count > soma::MAX_PORT_PUBLICATIONS {
            return Err(Error::Protocol("publication count"));
        }
        let mut publications = Vec::with_capacity(publication_count);
        for _ in 0..publication_count {
            publications.push(decode_publication(&mut reader)?);
        }
        if reader.cursor != bytes.len() {
            return Err(Error::Protocol("trailing bytes"));
        }
        Self::new(egress, resolvers, publications, profile)
    }
}

fn encode_publication(publication: &PortPublication, out: &mut Vec<u8>) {
    match publication.bind() {
        HostBind::Ipv4 { address } => {
            out.push(4);
            out.extend_from_slice(&address.to_ipv6_mapped().octets());
            out.push(0);
        }
        HostBind::Ipv6 { address, v6_only } => {
            out.push(6);
            out.extend_from_slice(&address.octets());
            out.push(u8::from(v6_only));
        }
    }
    let host_port = publication.host_port().requested().map_or(0, u16::from);
    out.extend_from_slice(&host_port.to_be_bytes());
    out.extend_from_slice(&publication.guest_port().get().to_be_bytes());
    out.push(match publication.protocol() {
        TransportProtocol::Tcp => 6,
        TransportProtocol::Udp => 17,
    });
}

fn decode_publication(reader: &mut Reader<'_>) -> Result<PortPublication, Error> {
    let family = reader.u8()?;
    let raw = Ipv6Addr::from(reader.array::<16>()?);
    let v6_only = reader.u8()?;
    let bind = match (family, v6_only) {
        (4, 0) => {
            let address = raw
                .to_ipv4_mapped()
                .ok_or(Error::Protocol("bind address"))?;
            HostBind::new(IpAddr::V4(address))
        }
        (6, 0 | 1) => HostBind::ipv6(raw, v6_only == 1),
        _ => return Err(Error::Protocol("bind family")),
    }
    .map_err(|_| Error::Protocol("bind"))?;
    let host_port = HostPort::from_u16(u16::from_be_bytes(reader.array()?));
    let guest_port = u16::from_be_bytes(reader.array()?);
    let protocol = match reader.u8()? {
        6 => TransportProtocol::Tcp,
        17 => TransportProtocol::Udp,
        _ => return Err(Error::Protocol("transport")),
    };
    PortPublication::new(bind, host_port, guest_port, protocol)
        .map_err(|_| Error::Protocol("publication"))
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl Reader<'_> {
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.array::<1>()?[0])
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let end = self
            .cursor
            .checked_add(N)
            .ok_or(Error::Protocol("length"))?;
        let slice = self
            .bytes
            .get(self.cursor..end)
            .ok_or(Error::Protocol("short"))?;
        self.cursor = end;
        let mut out = [0; N];
        out.copy_from_slice(slice);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent() -> NetworkIntent {
        let publications = vec![
            PortPublication::new(
                HostBind::loopback_v4(),
                HostPort::Automatic,
                8080,
                TransportProtocol::Tcp,
            )
            .expect("publication"),
            PortPublication::new(
                HostBind::ipv6(Ipv6Addr::LOCALHOST, true).expect("bind"),
                HostPort::from_u16(5353),
                53,
                TransportProtocol::Udp,
            )
            .expect("publication"),
        ];
        NetworkIntent::new(
            EgressClass::PublicInternet,
            vec![Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(9, 9, 9, 9)],
            publications,
            ProfileDigest([7; 32]),
        )
        .expect("intent")
    }

    #[test]
    fn intent_round_trips_and_rejects_hostile_bytes() {
        let encoded = intent().encode();
        assert!(encoded.len() <= MAX_ENCODED_INTENT);
        assert_eq!(NetworkIntent::decode(&encoded).expect("decodes"), intent());
        let mut trailing = encoded.clone();
        trailing.push(0);
        assert_eq!(
            NetworkIntent::decode(&trailing).expect_err("trailing"),
            Error::Protocol("trailing bytes")
        );
        assert_eq!(
            NetworkIntent::decode(&encoded[..encoded.len() - 1]).expect_err("short"),
            Error::Protocol("short")
        );
        let mut bad_egress = encoded.clone();
        bad_egress[0] = 9;
        assert_eq!(
            NetworkIntent::decode(&bad_egress).expect_err("egress"),
            Error::Protocol("egress")
        );
        let mut too_many = encoded;
        too_many[33] = 9;
        assert_eq!(
            NetworkIntent::decode(&too_many).expect_err("count"),
            Error::Protocol("resolver count")
        );
        for length in 0..MAX_ENCODED_INTENT {
            let _ = NetworkIntent::decode(&vec![0xff; length]);
        }
    }
}
