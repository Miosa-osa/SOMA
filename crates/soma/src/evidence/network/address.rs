use std::net::IpAddr;

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::ValidationError;

/// Maximum number of guest addresses retained in one execution receipt.
pub const MAX_ASSIGNED_ADDRESSES: usize = 16;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct AssignedAddress {
    address: IpAddr,
    prefix_length: u8,
}

impl AssignedAddress {
    /// Creates one validated guest network address.
    ///
    /// # Errors
    ///
    /// Rejects unspecified, multicast, and invalid-prefix addresses.
    pub fn new(address: IpAddr, prefix_length: u8) -> Result<Self, ValidationError> {
        let prefix_is_valid = match address {
            IpAddr::V4(_) => prefix_length <= 32,
            IpAddr::V6(_) => prefix_length <= 128,
        };
        if !prefix_is_valid || address.is_unspecified() || address.is_multicast() {
            return Err(ValidationError::InvalidNetworkPolicy);
        }
        Ok(Self {
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssignedAddressWire {
    address: IpAddr,
    prefix_length: u8,
}

impl<'de> Deserialize<'de> for AssignedAddress {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = AssignedAddressWire::deserialize(deserializer)?;
        Self::new(wire.address, wire.prefix_length).map_err(D::Error::custom)
    }
}
