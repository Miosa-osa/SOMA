use std::{net::Ipv4Addr, num::NonZeroU16};

use serde::Serialize;

use crate::{RequestError, RequestErrorReason};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportProtocol {
    Tcp,
    Udp,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct PublishedPort {
    host_address: Ipv4Addr,
    host_port: NonZeroU16,
    guest_port: NonZeroU16,
    protocol: TransportProtocol,
}

impl PublishedPort {
    /// Creates one concrete Apple Container IPv4 publication.
    ///
    /// # Errors
    ///
    /// Apple Container 1.3 rejects port zero and currently rejects port one.
    pub fn new(
        host_address: Ipv4Addr,
        host_port: u16,
        guest_port: u16,
        protocol: TransportProtocol,
    ) -> Result<Self, RequestError> {
        if host_address.is_multicast() || host_address.is_broadcast() {
            return Err(RequestError::new(
                "host_address",
                RequestErrorReason::InvalidCharacter,
            ));
        }
        let Some(host_port) = NonZeroU16::new(host_port).filter(|port| port.get() >= 2) else {
            return Err(RequestError::new(
                "published_port",
                RequestErrorReason::Zero,
            ));
        };
        let Some(guest_port) = NonZeroU16::new(guest_port).filter(|port| port.get() >= 2) else {
            return Err(RequestError::new(
                "published_port",
                RequestErrorReason::Zero,
            ));
        };
        Ok(Self {
            host_address,
            host_port,
            guest_port,
            protocol,
        })
    }

    #[must_use]
    pub const fn host_address(self) -> Ipv4Addr {
        self.host_address
    }

    #[must_use]
    pub const fn host_port(self) -> NonZeroU16 {
        self.host_port
    }

    #[must_use]
    pub const fn guest_port(self) -> NonZeroU16 {
        self.guest_port
    }

    #[must_use]
    pub const fn protocol(self) -> TransportProtocol {
        self.protocol
    }
}
