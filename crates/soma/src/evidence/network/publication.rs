use std::num::NonZeroU16;

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::{HostBind, HostPort, PortPublication, TransportProtocol, ValidationError};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct EffectivePortPublication {
    bind: HostBind,
    host_port: NonZeroU16,
    guest_port: NonZeroU16,
    protocol: TransportProtocol,
}

impl EffectivePortPublication {
    /// Creates an observed host-to-guest port mapping.
    ///
    /// # Errors
    ///
    /// Rejects a zero host port or zero guest port.
    pub fn new(
        bind: HostBind,
        host_port: u16,
        guest_port: u16,
        protocol: TransportProtocol,
    ) -> Result<Self, ValidationError> {
        PortPublication::new(bind, HostPort::from_u16(host_port), guest_port, protocol)?;
        Ok(Self {
            bind,
            host_port: NonZeroU16::new(host_port).ok_or(ValidationError::InvalidNetworkPolicy)?,
            guest_port: NonZeroU16::new(guest_port).ok_or(ValidationError::InvalidNetworkPolicy)?,
            protocol,
        })
    }

    #[must_use]
    pub const fn bind(&self) -> HostBind {
        self.bind
    }

    #[must_use]
    pub const fn host_port(&self) -> NonZeroU16 {
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
struct EffectivePortPublicationWire {
    bind: HostBind,
    host_port: u16,
    guest_port: u16,
    protocol: TransportProtocol,
}

impl<'de> Deserialize<'de> for EffectivePortPublication {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = EffectivePortPublicationWire::deserialize(deserializer)?;
        Self::new(wire.bind, wire.host_port, wire.guest_port, wire.protocol)
            .map_err(D::Error::custom)
    }
}
