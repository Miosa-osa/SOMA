use std::collections::BTreeSet;

use serde::Serialize;

use crate::{RequestError, RequestErrorReason};

use super::{DnsConfiguration, NetworkPolicy, PublishedPort};

pub const MAX_PUBLISHED_PORTS: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NetworkConfiguration {
    attachment: NetworkPolicy,
    dns: DnsConfiguration,
    published_ports: Vec<PublishedPort>,
}

impl NetworkConfiguration {
    #[must_use]
    pub const fn for_attachment(attachment: NetworkPolicy) -> Self {
        Self {
            attachment,
            dns: DnsConfiguration::RuntimeDefault,
            published_ports: Vec::new(),
        }
    }

    #[must_use]
    pub const fn runtime_default() -> Self {
        Self::for_attachment(NetworkPolicy::Unspecified)
    }

    #[must_use]
    pub const fn isolated() -> Self {
        Self::for_attachment(NetworkPolicy::Denied)
    }

    /// Creates one validated Apple Container network plan.
    ///
    /// # Errors
    ///
    /// Rejects publications or custom DNS without an explicit default-network attachment,
    /// oversized publication sets, duplicates, and fixed host endpoint collisions.
    pub fn new(
        attachment: NetworkPolicy,
        dns: DnsConfiguration,
        mut published_ports: Vec<PublishedPort>,
    ) -> Result<Self, RequestError> {
        if attachment != NetworkPolicy::Allowed
            && (dns != DnsConfiguration::RuntimeDefault || !published_ports.is_empty())
        {
            return Err(RequestError::new(
                "network",
                RequestErrorReason::InvalidCharacter,
            ));
        }
        if published_ports.len() > MAX_PUBLISHED_PORTS {
            return Err(RequestError::new(
                "published_ports",
                RequestErrorReason::TooLarge,
            ));
        }
        let endpoints = published_ports
            .iter()
            .map(|port| (port.host_address(), port.host_port(), port.protocol()))
            .collect::<BTreeSet<_>>();
        if endpoints.len() != published_ports.len() {
            return Err(RequestError::new(
                "published_ports",
                RequestErrorReason::InvalidIdentifier,
            ));
        }
        published_ports.sort_unstable();
        Ok(Self {
            attachment,
            dns,
            published_ports,
        })
    }

    #[must_use]
    pub const fn attachment(&self) -> NetworkPolicy {
        self.attachment
    }

    #[must_use]
    pub const fn dns(&self) -> &DnsConfiguration {
        &self.dns
    }

    #[must_use]
    pub fn published_ports(&self) -> &[PublishedPort] {
        &self.published_ports
    }
}
