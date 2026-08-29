use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::{DnsPolicy, EgressPolicy, MAX_PORT_PUBLICATIONS, NetworkPolicy, ValidationError};

use super::{
    AssignedAddress, EffectivePortPublication, MAX_ASSIGNED_ADDRESSES, PortActivationClass,
    matching::{
        activation_matches_publications, dns_matches, egress_matches, endpoint_collision,
        publications_match, requested_activation_matches,
    },
};
use crate::{Observation, ObservationUnavailable};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkAttachment {
    Detached,
    Attached,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EffectiveNetwork {
    attachment: Observation<NetworkAttachment>,
    egress: Observation<EgressPolicy>,
    dns: Observation<DnsPolicy>,
    addresses: Observation<Vec<AssignedAddress>>,
    published_ports: Observation<Vec<EffectivePortPublication>>,
    port_activation: Observation<PortActivationClass>,
}

impl EffectiveNetwork {
    /// Creates validated network evidence from independent observations.
    ///
    /// # Errors
    ///
    /// Rejects non-concrete observations, oversized publication sets, and host endpoint
    /// collisions.
    pub fn new(
        attachment: Observation<NetworkAttachment>,
        egress: Observation<EgressPolicy>,
        dns: Observation<DnsPolicy>,
        mut addresses: Observation<Vec<AssignedAddress>>,
        mut published_ports: Observation<Vec<EffectivePortPublication>>,
        port_activation: Observation<PortActivationClass>,
    ) -> Result<Self, ValidationError> {
        if matches!(egress, Observation::Observed(EgressPolicy::Unspecified))
            || matches!(dns, Observation::Observed(DnsPolicy::Unspecified))
        {
            return Err(ValidationError::InvalidNetworkPolicy);
        }
        if let Observation::Observed(values) = &mut addresses {
            values.sort_unstable();
            if values.len() > MAX_ASSIGNED_ADDRESSES
                || values.windows(2).any(|pair| pair[0] == pair[1])
            {
                return Err(ValidationError::InvalidNetworkPolicy);
            }
        }
        if let Observation::Observed(publications) = &mut published_ports {
            publications.sort_unstable();
            if publications.len() > MAX_PORT_PUBLICATIONS
                || publications.windows(2).any(|pair| pair[0] == pair[1])
                || endpoint_collision(publications)
            {
                return Err(ValidationError::InvalidNetworkPolicy);
            }
        }
        if !activation_matches_publications(&published_ports, &port_activation) {
            return Err(ValidationError::InvalidNetworkPolicy);
        }
        Ok(Self {
            attachment,
            egress,
            dns,
            addresses,
            published_ports,
            port_activation,
        })
    }

    #[must_use]
    pub fn unavailable(reason: ObservationUnavailable) -> Self {
        Self {
            attachment: Observation::Unavailable(reason),
            egress: Observation::Unavailable(reason),
            dns: Observation::Unavailable(reason),
            addresses: Observation::Unavailable(reason),
            published_ports: Observation::Unavailable(reason),
            port_activation: Observation::Unavailable(reason),
        }
    }

    #[must_use]
    pub const fn attachment(&self) -> &Observation<NetworkAttachment> {
        &self.attachment
    }

    #[must_use]
    pub const fn egress(&self) -> &Observation<EgressPolicy> {
        &self.egress
    }

    #[must_use]
    pub const fn dns(&self) -> &Observation<DnsPolicy> {
        &self.dns
    }

    #[must_use]
    pub const fn addresses(&self) -> &Observation<Vec<AssignedAddress>> {
        &self.addresses
    }

    #[must_use]
    pub const fn published_ports(&self) -> &Observation<Vec<EffectivePortPublication>> {
        &self.published_ports
    }

    #[must_use]
    pub const fn port_activation(&self) -> &Observation<PortActivationClass> {
        &self.port_activation
    }

    pub(crate) fn matches_request(&self, requested: &NetworkPolicy) -> bool {
        egress_matches(&self.egress, requested.egress())
            && dns_matches(&self.dns, requested.dns())
            && publications_match(&self.published_ports, requested.published_ports())
            && requested_activation_matches(&self.port_activation, requested.published_ports())
    }

    pub(crate) fn all_unavailable(&self) -> bool {
        matches!(self.attachment, Observation::Unavailable(_))
            && matches!(self.egress, Observation::Unavailable(_))
            && matches!(self.dns, Observation::Unavailable(_))
            && matches!(self.addresses, Observation::Unavailable(_))
            && matches!(self.published_ports, Observation::Unavailable(_))
            && matches!(self.port_activation, Observation::Unavailable(_))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectiveNetworkWire {
    attachment: Observation<NetworkAttachment>,
    egress: Observation<EgressPolicy>,
    dns: Observation<DnsPolicy>,
    addresses: Observation<Vec<AssignedAddress>>,
    published_ports: Observation<Vec<EffectivePortPublication>>,
    port_activation: Observation<PortActivationClass>,
}

impl<'de> Deserialize<'de> for EffectiveNetwork {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = EffectiveNetworkWire::deserialize(deserializer)?;
        Self::new(
            wire.attachment,
            wire.egress,
            wire.dns,
            wire.addresses,
            wire.published_ports,
            wire.port_activation,
        )
        .map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests;
