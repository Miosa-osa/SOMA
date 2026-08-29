use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    num::NonZeroU16,
};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::ValidationError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportProtocol {
    Tcp,
    Udp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "mode", content = "port", rename_all = "snake_case")]
pub enum HostPort {
    Automatic,
    Fixed(NonZeroU16),
}

impl HostPort {
    #[must_use]
    pub const fn from_u16(port: u16) -> Self {
        match NonZeroU16::new(port) {
            Some(port) => Self::Fixed(port),
            None => Self::Automatic,
        }
    }

    #[must_use]
    pub const fn requested(self) -> Option<NonZeroU16> {
        match self {
            Self::Automatic => None,
            Self::Fixed(port) => Some(port),
        }
    }
}

/// An explicit host bind target.
///
/// IPv6 behavior is part of the request because an IPv6 wildcard may otherwise also consume the
/// IPv4 endpoint on hosts where dual-stack sockets are enabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum HostBind {
    Ipv4 { address: Ipv4Addr },
    Ipv6 { address: Ipv6Addr, v6_only: bool },
}

impl HostBind {
    /// Creates a validated bind with the safer IPv6-only behavior for IPv6 addresses.
    ///
    /// # Errors
    ///
    /// Rejects multicast and IPv4 broadcast addresses.
    pub fn new(address: IpAddr) -> Result<Self, ValidationError> {
        match address {
            IpAddr::V4(address) => Self::ipv4(address),
            IpAddr::V6(address) => Self::ipv6(address, true),
        }
    }

    /// Creates a validated IPv4 bind.
    ///
    /// # Errors
    ///
    /// Rejects multicast and broadcast addresses.
    pub fn ipv4(address: Ipv4Addr) -> Result<Self, ValidationError> {
        if address.is_multicast() || address.is_broadcast() {
            return Err(ValidationError::InvalidNetworkPolicy);
        }
        Ok(Self::Ipv4 { address })
    }

    /// Creates a validated IPv6 bind with explicit dual-stack behavior.
    ///
    /// # Errors
    ///
    /// Rejects multicast addresses.
    pub fn ipv6(address: Ipv6Addr, v6_only: bool) -> Result<Self, ValidationError> {
        if address.is_multicast() {
            return Err(ValidationError::InvalidNetworkPolicy);
        }
        Ok(Self::Ipv6 { address, v6_only })
    }

    #[must_use]
    pub const fn loopback_v4() -> Self {
        Self::Ipv4 {
            address: Ipv4Addr::LOCALHOST,
        }
    }

    #[must_use]
    pub const fn address(self) -> IpAddr {
        match self {
            Self::Ipv4 { address } => IpAddr::V4(address),
            Self::Ipv6 { address, .. } => IpAddr::V6(address),
        }
    }

    #[must_use]
    pub const fn v6_only(self) -> Option<bool> {
        match self {
            Self::Ipv4 { .. } => None,
            Self::Ipv6 { v6_only, .. } => Some(v6_only),
        }
    }

    pub(crate) fn conflicts(self, other: Self) -> bool {
        match (self, other) {
            (Self::Ipv4 { address: left }, Self::Ipv4 { address: right }) => {
                left.is_unspecified() || right.is_unspecified() || left == right
            }
            (Self::Ipv6 { address: left, .. }, Self::Ipv6 { address: right, .. }) => {
                left.is_unspecified() || right.is_unspecified() || left == right
            }
            (Self::Ipv6 { address, v6_only }, Self::Ipv4 { .. })
            | (Self::Ipv4 { .. }, Self::Ipv6 { address, v6_only }) => {
                address.is_unspecified() && !v6_only
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "family", rename_all = "snake_case", deny_unknown_fields)]
enum HostBindWire {
    Ipv4 { address: Ipv4Addr },
    Ipv6 { address: Ipv6Addr, v6_only: bool },
}

impl<'de> Deserialize<'de> for HostBind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match HostBindWire::deserialize(deserializer)? {
            HostBindWire::Ipv4 { address } => Self::ipv4(address).map_err(D::Error::custom),
            HostBindWire::Ipv6 { address, v6_only } => {
                Self::ipv6(address, v6_only).map_err(D::Error::custom)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct PortPublication {
    bind: HostBind,
    host_port: HostPort,
    guest_port: NonZeroU16,
    protocol: TransportProtocol,
}

impl PortPublication {
    /// Creates one explicit host-to-guest port publication.
    ///
    /// Port zero requests an automatically allocated host port.
    ///
    /// # Errors
    ///
    /// Rejects a zero guest port.
    pub fn new(
        bind: HostBind,
        host_port: HostPort,
        guest_port: u16,
        protocol: TransportProtocol,
    ) -> Result<Self, ValidationError> {
        Ok(Self {
            bind,
            host_port,
            guest_port: NonZeroU16::new(guest_port).ok_or(ValidationError::InvalidNetworkPolicy)?,
            protocol,
        })
    }

    /// Creates a loopback-only TCP publication with automatic host-port allocation.
    ///
    /// # Errors
    ///
    /// Rejects a zero guest port.
    pub fn loopback_tcp(guest_port: u16) -> Result<Self, ValidationError> {
        Self::new(
            HostBind::loopback_v4(),
            HostPort::Automatic,
            guest_port,
            TransportProtocol::Tcp,
        )
    }

    #[must_use]
    pub const fn bind(&self) -> HostBind {
        self.bind
    }

    #[must_use]
    pub const fn host_port(&self) -> HostPort {
        self.host_port
    }

    #[must_use]
    pub const fn guest_port(&self) -> NonZeroU16 {
        self.guest_port
    }

    #[must_use]
    pub const fn protocol(&self) -> TransportProtocol {
        self.protocol
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PortPublicationWire {
    bind: HostBind,
    host_port: HostPort,
    guest_port: u16,
    protocol: TransportProtocol,
}

impl<'de> Deserialize<'de> for PortPublication {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = PortPublicationWire::deserialize(deserializer)?;
        Self::new(wire.bind, wire.host_port, wire.guest_port, wire.protocol)
            .map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::{HostBind, HostPort, PortPublication, TransportProtocol};

    #[test]
    fn default_publication_is_loopback_tcp_with_automatic_host_port() {
        let publication = PortPublication::loopback_tcp(8_080).expect("valid publication");

        assert_eq!(publication.bind(), HostBind::loopback_v4());
        assert_eq!(publication.host_port(), HostPort::Automatic);
        assert_eq!(publication.guest_port().get(), 8_080);
        assert_eq!(publication.protocol(), TransportProtocol::Tcp);
    }

    #[test]
    fn dual_stack_wildcard_conflicts_with_ipv4_binds() {
        let dual_stack = HostBind::ipv6(Ipv6Addr::UNSPECIFIED, false).expect("IPv6 bind");
        let v4 = HostBind::new(IpAddr::V4(Ipv4Addr::LOCALHOST)).expect("IPv4 bind");
        let v6_only = HostBind::ipv6(Ipv6Addr::UNSPECIFIED, true).expect("IPv6-only bind");

        assert!(dual_stack.conflicts(v4));
        assert!(!v6_only.conflicts(v4));
    }

    #[test]
    fn deserialization_rejects_zero_guest_port() {
        let zero_guest = r#"{"bind":{"family":"ipv4","address":"127.0.0.1"},"host_port":{"mode":"automatic"},"guest_port":0,"protocol":"tcp"}"#;

        assert!(serde_json::from_str::<PortPublication>(zero_guest).is_err());
    }
}
