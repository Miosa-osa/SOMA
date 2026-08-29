use std::{collections::BTreeSet, net::IpAddr};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::ValidationError;

/// Maximum number of DNS resolvers accepted in one portable request.
pub const MAX_DNS_SERVERS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum DnsPolicy {
    /// Leave resolver configuration to the backend and require an honest observation.
    Unspecified,
    /// Make name resolution unavailable and prevent DNS flows.
    Denied,
    /// Use the backend's isolation-aware managed resolver.
    System,
    /// Use exactly the supplied resolver addresses.
    Custom { servers: Vec<IpAddr> },
}

impl DnsPolicy {
    /// Creates a bounded, canonical custom resolver policy.
    ///
    /// # Errors
    ///
    /// Rejects an empty, oversized, duplicate, unspecified, multicast, or IPv4 broadcast set.
    pub fn custom(mut servers: Vec<IpAddr>) -> Result<Self, ValidationError> {
        if servers.is_empty()
            || servers.len() > MAX_DNS_SERVERS
            || servers.iter().any(invalid_resolver)
            || servers.iter().collect::<BTreeSet<_>>().len() != servers.len()
        {
            return Err(ValidationError::InvalidNetworkPolicy);
        }
        servers.sort_unstable();
        Ok(Self::Custom { servers })
    }

    #[must_use]
    pub fn servers(&self) -> &[IpAddr] {
        match self {
            Self::Custom { servers } => servers,
            Self::Unspecified | Self::Denied | Self::System => &[],
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum DnsPolicyWire {
    Unspecified,
    Denied,
    System,
    Custom { servers: Vec<IpAddr> },
}

impl<'de> Deserialize<'de> for DnsPolicy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match DnsPolicyWire::deserialize(deserializer)? {
            DnsPolicyWire::Unspecified => Ok(Self::Unspecified),
            DnsPolicyWire::Denied => Ok(Self::Denied),
            DnsPolicyWire::System => Ok(Self::System),
            DnsPolicyWire::Custom { servers } => Self::custom(servers).map_err(D::Error::custom),
        }
    }
}

fn invalid_resolver(address: &IpAddr) -> bool {
    address.is_unspecified()
        || address.is_multicast()
        || matches!(address, IpAddr::V4(value) if value.is_broadcast())
}

#[cfg(test)]
mod tests {
    use super::DnsPolicy;

    #[test]
    fn deserialization_revalidates_custom_dns() {
        let duplicate = r#"{"mode":"custom","servers":["1.1.1.1","1.1.1.1"]}"#;

        assert!(serde_json::from_str::<DnsPolicy>(duplicate).is_err());
    }
}
