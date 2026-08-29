use std::net::IpAddr;

use serde::Serialize;

use crate::PublishedPort;

/// Network attachment observed in both the configured and active runtime documents.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkAttachment {
    Detached,
    Attached,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct NetworkAddress {
    address: IpAddr,
    prefix_length: u8,
}

impl NetworkAddress {
    pub(crate) const fn new(address: IpAddr, prefix_length: u8) -> Option<Self> {
        let valid_prefix = match address {
            IpAddr::V4(_) => prefix_length <= 32,
            IpAddr::V6(_) => prefix_length <= 128,
        };
        if !valid_prefix || address.is_unspecified() || address.is_multicast() {
            return None;
        }
        Some(Self {
            address,
            prefix_length,
        })
    }

    #[must_use]
    pub const fn address(self) -> IpAddr {
        self.address
    }

    #[must_use]
    pub const fn prefix_length(self) -> u8 {
        self.prefix_length
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InspectedNetwork {
    attachment: Option<NetworkAttachment>,
    dns_servers: Option<Vec<IpAddr>>,
    published_ports: Option<Vec<PublishedPort>>,
    addresses: Option<Vec<NetworkAddress>>,
}

impl InspectedNetwork {
    pub(crate) fn new(
        attachment: Option<NetworkAttachment>,
        dns_servers: Option<Vec<IpAddr>>,
        published_ports: Option<Vec<PublishedPort>>,
        addresses: Option<Vec<NetworkAddress>>,
    ) -> Self {
        Self {
            attachment,
            dns_servers,
            published_ports,
            addresses,
        }
    }

    #[must_use]
    pub const fn attachment(&self) -> Option<NetworkAttachment> {
        self.attachment
    }

    #[must_use]
    pub fn dns_servers(&self) -> Option<&[IpAddr]> {
        self.dns_servers.as_deref()
    }

    #[must_use]
    pub fn published_ports(&self) -> Option<&[PublishedPort]> {
        self.published_ports.as_deref()
    }

    #[must_use]
    pub fn addresses(&self) -> Option<&[NetworkAddress]> {
        self.addresses.as_deref()
    }
}
