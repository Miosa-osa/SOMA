use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::ValidationError;

/// Optional human-readable metadata for a machine.
///
/// A name is never an ownership or routing identity. Backend operations always
/// address machines with an [`crate::InstanceId`].
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct MachineName(String);

impl MachineName {
    pub const MIN_BYTES: usize = 1;
    pub const MAX_BYTES: usize = 63;

    /// Parses optional human metadata that is never used as machine identity.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidMachineName`] unless the value is 1 to 63 lowercase
    /// ASCII alphanumeric or hyphen bytes with alphanumeric ends.
    pub fn parse(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        let bytes = value.as_bytes();
        let valid_character =
            |byte: &u8| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-';
        if bytes.len() < Self::MIN_BYTES
            || bytes.len() > Self::MAX_BYTES
            || !bytes.iter().all(valid_character)
            || !bytes.first().is_some_and(u8::is_ascii_alphanumeric)
            || !bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        {
            return Err(ValidationError::InvalidMachineName);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for MachineName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl fmt::Debug for MachineName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MachineName")
            .field("bytes", &self.0.len())
            .finish()
    }
}
