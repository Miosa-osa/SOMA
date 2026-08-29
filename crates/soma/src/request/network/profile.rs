use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::ValidationError;

macro_rules! profile_id {
    ($name:ident) => {
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Parses one canonical operator-owned profile identifier.
            ///
            /// # Errors
            ///
            /// Rejects values outside the 1 to 63 character lowercase portable grammar.
            pub fn parse(value: impl Into<String>) -> Result<Self, ValidationError> {
                let value = value.into();
                if !valid_profile_id(&value) {
                    return Err(ValidationError::InvalidNetworkPolicy);
                }
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_tuple(stringify!($name))
                    .field(&self.0)
                    .finish()
            }
        }
    };
}

profile_id!(NetworkProfileId);
profile_id!(ProxyProfileId);

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ProfileRevision(String);

impl ProfileRevision {
    /// Parses one canonical SHA-256 profile revision.
    ///
    /// # Errors
    ///
    /// Rejects any value other than `sha256:` followed by 64 lowercase hexadecimal characters.
    pub fn parse(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        let valid = value.strip_prefix("sha256:").is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        });
        if !valid {
            return Err(ValidationError::InvalidNetworkPolicy);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ProfileRevision {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl fmt::Debug for ProfileRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ProfileRevision")
            .field(&self.0)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum NetworkProfileSelector {
    Disabled,
    OperatorDefault,
    Named {
        profile_id: NetworkProfileId,
        revision: ProfileRevision,
    },
}

impl NetworkProfileSelector {
    #[must_use]
    pub const fn disabled() -> Self {
        Self::Disabled
    }

    #[must_use]
    pub const fn operator_default() -> Self {
        Self::OperatorDefault
    }

    #[must_use]
    pub const fn named(profile_id: NetworkProfileId, revision: ProfileRevision) -> Self {
        Self::Named {
            profile_id,
            revision,
        }
    }

    #[must_use]
    pub const fn is_operator_default(&self) -> bool {
        matches!(self, Self::OperatorDefault)
    }

    #[must_use]
    pub const fn is_disabled(&self) -> bool {
        matches!(self, Self::Disabled)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProxyProfileSelector {
    OperatorDefault,
    Named {
        profile_id: ProxyProfileId,
        revision: ProfileRevision,
    },
}

impl ProxyProfileSelector {
    #[must_use]
    pub const fn operator_default() -> Self {
        Self::OperatorDefault
    }

    #[must_use]
    pub const fn named(profile_id: ProxyProfileId, revision: ProfileRevision) -> Self {
        Self::Named {
            profile_id,
            revision,
        }
    }
}

fn valid_profile_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=63).contains(&bytes.len())
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}
