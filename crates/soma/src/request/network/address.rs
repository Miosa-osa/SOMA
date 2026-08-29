use std::net::{Ipv4Addr, Ipv6Addr};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::ValidationError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Ipv4AddressIntent(Ipv4AddressMode);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    content = "address",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum Ipv4AddressMode {
    Unspecified,
    Disabled,
    Allocated,
    Requested(Ipv4Addr),
}

impl Ipv4AddressIntent {
    #[must_use]
    pub const fn unspecified() -> Self {
        Self(Ipv4AddressMode::Unspecified)
    }

    #[must_use]
    pub const fn disabled() -> Self {
        Self(Ipv4AddressMode::Disabled)
    }

    #[must_use]
    pub const fn allocated() -> Self {
        Self(Ipv4AddressMode::Allocated)
    }

    /// Requests one exact guest IPv4 address.
    ///
    /// # Errors
    ///
    /// Rejects unspecified, loopback, multicast, and broadcast addresses.
    pub fn requested(address: Ipv4Addr) -> Result<Self, ValidationError> {
        if invalid_ipv4(address) {
            return Err(ValidationError::InvalidNetworkPolicy);
        }
        Ok(Self(Ipv4AddressMode::Requested(address)))
    }

    #[must_use]
    pub const fn is_unspecified(&self) -> bool {
        matches!(self.0, Ipv4AddressMode::Unspecified)
    }

    #[must_use]
    pub const fn is_disabled(&self) -> bool {
        matches!(self.0, Ipv4AddressMode::Disabled)
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        matches!(
            self.0,
            Ipv4AddressMode::Allocated | Ipv4AddressMode::Requested(_)
        )
    }

    #[must_use]
    pub const fn requested_address(&self) -> Option<Ipv4Addr> {
        match self.0 {
            Ipv4AddressMode::Requested(address) => Some(address),
            Ipv4AddressMode::Unspecified
            | Ipv4AddressMode::Disabled
            | Ipv4AddressMode::Allocated => None,
        }
    }

    pub(crate) const fn fingerprint_code(self) -> u8 {
        match self.0 {
            Ipv4AddressMode::Unspecified => 0,
            Ipv4AddressMode::Disabled => 1,
            Ipv4AddressMode::Allocated => 2,
            Ipv4AddressMode::Requested(_) => 3,
        }
    }
}

impl<'de> Deserialize<'de> for Ipv4AddressIntent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Ipv4AddressMode::deserialize(deserializer)? {
            Ipv4AddressMode::Unspecified => Ok(Self::unspecified()),
            Ipv4AddressMode::Disabled => Ok(Self::disabled()),
            Ipv4AddressMode::Allocated => Ok(Self::allocated()),
            Ipv4AddressMode::Requested(address) => {
                Self::requested(address).map_err(D::Error::custom)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Ipv6AddressIntent(Ipv6AddressMode);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    content = "address",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum Ipv6AddressMode {
    Unspecified,
    Disabled,
    Allocated,
    Requested(Ipv6Addr),
}

impl Ipv6AddressIntent {
    #[must_use]
    pub const fn unspecified() -> Self {
        Self(Ipv6AddressMode::Unspecified)
    }

    #[must_use]
    pub const fn disabled() -> Self {
        Self(Ipv6AddressMode::Disabled)
    }

    #[must_use]
    pub const fn allocated() -> Self {
        Self(Ipv6AddressMode::Allocated)
    }

    /// Requests one exact guest IPv6 address.
    ///
    /// # Errors
    ///
    /// Rejects unspecified, loopback, multicast, and link-local addresses.
    pub fn requested(address: Ipv6Addr) -> Result<Self, ValidationError> {
        if invalid_ipv6(address) {
            return Err(ValidationError::InvalidNetworkPolicy);
        }
        Ok(Self(Ipv6AddressMode::Requested(address)))
    }

    #[must_use]
    pub const fn is_unspecified(&self) -> bool {
        matches!(self.0, Ipv6AddressMode::Unspecified)
    }

    #[must_use]
    pub const fn is_disabled(&self) -> bool {
        matches!(self.0, Ipv6AddressMode::Disabled)
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        matches!(
            self.0,
            Ipv6AddressMode::Allocated | Ipv6AddressMode::Requested(_)
        )
    }

    #[must_use]
    pub const fn requested_address(&self) -> Option<Ipv6Addr> {
        match self.0 {
            Ipv6AddressMode::Requested(address) => Some(address),
            Ipv6AddressMode::Unspecified
            | Ipv6AddressMode::Disabled
            | Ipv6AddressMode::Allocated => None,
        }
    }

    pub(crate) const fn fingerprint_code(self) -> u8 {
        match self.0 {
            Ipv6AddressMode::Unspecified => 0,
            Ipv6AddressMode::Disabled => 1,
            Ipv6AddressMode::Allocated => 2,
            Ipv6AddressMode::Requested(_) => 3,
        }
    }
}

impl<'de> Deserialize<'de> for Ipv6AddressIntent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Ipv6AddressMode::deserialize(deserializer)? {
            Ipv6AddressMode::Unspecified => Ok(Self::unspecified()),
            Ipv6AddressMode::Disabled => Ok(Self::disabled()),
            Ipv6AddressMode::Allocated => Ok(Self::allocated()),
            Ipv6AddressMode::Requested(address) => {
                Self::requested(address).map_err(D::Error::custom)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct GuestAddressIntent {
    ipv4: Ipv4AddressIntent,
    ipv6: Ipv6AddressIntent,
}

impl GuestAddressIntent {
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            ipv4: Ipv4AddressIntent::disabled(),
            ipv6: Ipv6AddressIntent::disabled(),
        }
    }

    #[must_use]
    pub const fn runtime_default() -> Self {
        Self {
            ipv4: Ipv4AddressIntent::unspecified(),
            ipv6: Ipv6AddressIntent::unspecified(),
        }
    }

    /// Creates an explicit, family-aware guest address request.
    ///
    /// # Errors
    ///
    /// Rejects a partial runtime-default request where only one family is unspecified.
    pub fn new(ipv4: Ipv4AddressIntent, ipv6: Ipv6AddressIntent) -> Result<Self, ValidationError> {
        if ipv4.is_unspecified() != ipv6.is_unspecified() {
            return Err(ValidationError::InvalidNetworkPolicy);
        }
        Ok(Self { ipv4, ipv6 })
    }

    #[must_use]
    pub const fn ipv4(&self) -> &Ipv4AddressIntent {
        &self.ipv4
    }

    #[must_use]
    pub const fn ipv6(&self) -> &Ipv6AddressIntent {
        &self.ipv6
    }

    #[must_use]
    pub const fn is_runtime_default(&self) -> bool {
        self.ipv4.is_unspecified() && self.ipv6.is_unspecified()
    }

    #[must_use]
    pub const fn any_enabled(&self) -> bool {
        self.ipv4.is_enabled() || self.ipv6.is_enabled()
    }

    #[must_use]
    pub const fn all_disabled(&self) -> bool {
        self.ipv4.is_disabled() && self.ipv6.is_disabled()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuestAddressIntentWire {
    ipv4: Ipv4AddressIntent,
    ipv6: Ipv6AddressIntent,
}

impl<'de> Deserialize<'de> for GuestAddressIntent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = GuestAddressIntentWire::deserialize(deserializer)?;
        Self::new(wire.ipv4, wire.ipv6).map_err(D::Error::custom)
    }
}

fn invalid_ipv4(address: Ipv4Addr) -> bool {
    address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || address.is_broadcast()
}

fn invalid_ipv6(address: Ipv6Addr) -> bool {
    address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || address.is_unicast_link_local()
}
