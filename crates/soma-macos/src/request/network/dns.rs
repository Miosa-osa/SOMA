use std::{collections::BTreeSet, net::IpAddr};

use serde::Serialize;

use crate::{RequestError, RequestErrorReason};

pub const MAX_DNS_SERVERS: usize = 8;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum DnsConfiguration {
    RuntimeDefault,
    Custom { servers: Vec<IpAddr> },
}

impl DnsConfiguration {
    /// Creates an exact, bounded resolver list.
    ///
    /// # Errors
    ///
    /// Rejects empty, oversized, duplicate, unspecified, multicast, and broadcast addresses.
    pub fn custom(mut servers: Vec<IpAddr>) -> Result<Self, RequestError> {
        if servers.is_empty() {
            return Err(RequestError::new("dns_servers", RequestErrorReason::Empty));
        }
        if servers.len() > MAX_DNS_SERVERS {
            return Err(RequestError::new(
                "dns_servers",
                RequestErrorReason::TooLarge,
            ));
        }
        if servers.iter().any(invalid_server)
            || servers.iter().collect::<BTreeSet<_>>().len() != servers.len()
        {
            return Err(RequestError::new(
                "dns_servers",
                RequestErrorReason::InvalidCharacter,
            ));
        }
        servers.sort_unstable();
        Ok(Self::Custom { servers })
    }

    #[must_use]
    pub fn servers(&self) -> &[IpAddr] {
        match self {
            Self::RuntimeDefault => &[],
            Self::Custom { servers } => servers,
        }
    }
}

fn invalid_server(address: &IpAddr) -> bool {
    address.is_unspecified()
        || address.is_multicast()
        || matches!(address, IpAddr::V4(value) if value.is_broadcast())
}
