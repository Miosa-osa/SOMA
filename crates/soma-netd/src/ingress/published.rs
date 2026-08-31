//! The destination mapping one reserved host port needs before the guest can be reached.
//!
//! A reservation only proves that nothing else owns the host endpoint. This type is the
//! second half: the exact host endpoint, the exact guest endpoint, and the transport, in the
//! form the nftables generators consume. It exists separately from [`super::PortReservation`]
//! because the reservation owns a socket and must not be cloned, while a mapping is a plain
//! value that activation, release, and evidence all copy freely.

use std::net::Ipv4Addr;

use soma::TransportProtocol;

/// One host endpoint mapped onto one guest endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublishedPort {
    bind: Option<Ipv4Addr>,
    host_port: u16,
    guest: Ipv4Addr,
    guest_port: u16,
    protocol: TransportProtocol,
}

impl PublishedPort {
    /// Builds one mapping; `bind` is `None` when the request named the IPv4 wildcard.
    #[must_use]
    pub const fn new(
        bind: Option<Ipv4Addr>,
        host_port: u16,
        guest: Ipv4Addr,
        guest_port: u16,
        protocol: TransportProtocol,
    ) -> Self {
        Self {
            bind,
            host_port,
            guest,
            guest_port,
            protocol,
        }
    }

    /// Returns the host address the request named, or `None` for the wildcard.
    #[must_use]
    pub const fn bind(&self) -> Option<Ipv4Addr> {
        self.bind
    }

    /// Returns the host port actually held.
    #[must_use]
    pub const fn host_port(&self) -> u16 {
        self.host_port
    }

    /// Returns the guest port the mapping delivers to.
    #[must_use]
    pub const fn guest_port(&self) -> u16 {
        self.guest_port
    }

    /// Returns whether the host endpoint is a loopback address.
    ///
    /// A loopback publication needs `route_localnet` on the bundle's host veth, because once
    /// conntrack has reversed the translation the reply arrives on that link carrying a
    /// `127.0.0.0/8` destination, which the kernel treats as a martian by default.
    #[must_use]
    pub fn binds_loopback(&self) -> bool {
        self.bind.is_some_and(|address| address.is_loopback())
    }

    /// Returns the nftables keyword of the transport.
    #[must_use]
    pub const fn transport(&self) -> &'static str {
        match self.protocol {
            TransportProtocol::Tcp => "tcp",
            TransportProtocol::Udp => "udp",
        }
    }

    /// Renders the match that selects packets arriving for this host endpoint.
    ///
    /// The wildcard bind deliberately omits any address match rather than naming `0.0.0.0`,
    /// which as a destination would match nothing.
    #[must_use]
    pub fn host_match(&self) -> String {
        let transport = self.transport();
        let port = self.host_port;
        match self.bind {
            Some(address) => format!("ip daddr {address} {transport} dport {port}"),
            None => format!("meta nfproto ipv4 {transport} dport {port}"),
        }
    }

    /// Renders the destination translation this mapping installs.
    #[must_use]
    pub fn translation(&self) -> String {
        format!("dnat ip to {}:{}", self.guest, self.guest_port)
    }

    /// Renders the match that selects packets already translated toward the guest endpoint.
    #[must_use]
    pub fn guest_match(&self) -> String {
        format!(
            "ip daddr {} {} dport {}",
            self.guest,
            self.transport(),
            self.guest_port
        )
    }

    /// Renders the match that selects the guest's replies on this endpoint.
    #[must_use]
    pub fn reply_match(&self) -> String {
        format!(
            "ip saddr {} {} sport {}",
            self.guest,
            self.transport(),
            self.guest_port
        )
    }

    /// Renders the operator-facing description used in activation and release evidence.
    #[must_use]
    pub fn describe(&self) -> String {
        let bind = self
            .bind
            .map_or_else(|| "*".to_owned(), |address| address.to_string());
        format!(
            "{bind}:{} -> {}:{} {}",
            self.host_port,
            self.guest,
            self.guest_port,
            self.transport()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUEST: Ipv4Addr = Ipv4Addr::new(10, 200, 0, 2);

    #[test]
    fn a_wildcard_bind_matches_on_family_rather_than_on_the_unspecified_address() {
        let wildcard = PublishedPort::new(None, 8080, GUEST, 80, TransportProtocol::Tcp);
        assert_eq!(wildcard.host_match(), "meta nfproto ipv4 tcp dport 8080");
        assert!(!wildcard.binds_loopback());
        assert_eq!(wildcard.describe(), "*:8080 -> 10.200.0.2:80 tcp");
    }

    #[test]
    fn a_loopback_bind_is_recognised_and_renders_every_match_it_needs() {
        let loopback = PublishedPort::new(
            Some(Ipv4Addr::LOCALHOST),
            41000,
            GUEST,
            53,
            TransportProtocol::Udp,
        );
        assert!(loopback.binds_loopback());
        assert_eq!(loopback.host_match(), "ip daddr 127.0.0.1 udp dport 41000");
        assert_eq!(loopback.translation(), "dnat ip to 10.200.0.2:53");
        assert_eq!(loopback.guest_match(), "ip daddr 10.200.0.2 udp dport 53");
        assert_eq!(loopback.reply_match(), "ip saddr 10.200.0.2 udp sport 53");
    }
}
