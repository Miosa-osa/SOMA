use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::request::ValidationError;

macro_rules! canonical_runtime_id {
    ($name:ident, $label:literal) => {
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// How many characters one of these identifiers is, always.
            ///
            /// A caller that has to size something around an identity reads it here rather than
            /// writing the number again beside its own reason for needing it.
            pub const LENGTH: usize = 32;

            /// Parses one canonical portable runtime identifier.
            ///
            /// # Errors
            ///
            /// Returns [`ValidationError::InvalidIdentity`] unless the value is exactly
            /// [`Self::LENGTH`] nonzero lowercase hexadecimal characters.
            pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
                let value = value.into();
                validate_fixed_hex(&value, Self::LENGTH, true)?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.debug_tuple($label).field(&self.0).finish()
            }
        }
    };
}

canonical_runtime_id!(OperationId, "OperationId");
canonical_runtime_id!(InstanceId, "InstanceId");

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct GenerationId(String);

impl GenerationId {
    /// Parses a content-addressed generation identity.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidIdentity`] unless the value is a nonzero canonical
    /// SHA-256 digest.
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(ValidationError::InvalidIdentity);
        };
        validate_fixed_hex(hex, 64, true)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for GenerationId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl fmt::Debug for GenerationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("GenerationId")
            .field(&self.0)
            .finish()
    }
}

fn validate_fixed_hex(
    value: &str,
    expected_length: usize,
    reject_zero: bool,
) -> Result<(), ValidationError> {
    if value.len() != expected_length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || (reject_zero && value.bytes().all(|byte| byte == b'0'))
    {
        return Err(ValidationError::InvalidIdentity);
    }
    Ok(())
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RequestFingerprint(String);

impl RequestFingerprint {
    pub(crate) fn from_digest(bytes: [u8; 32]) -> Self {
        let mut encoded = String::with_capacity(71);
        encoded.push_str("sha256:");
        for byte in bytes {
            use std::fmt::Write as _;
            write!(encoded, "{byte:02x}").expect("writing to a String cannot fail");
        }
        Self(encoded)
    }

    /// Validates a canonical SHA-256 request fingerprint.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidIdentity`] for any noncanonical digest.
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(ValidationError::InvalidIdentity);
        };
        validate_fixed_hex(hex, 64, false)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for RequestFingerprint {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl fmt::Debug for RequestFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("RequestFingerprint")
            .field(&self.0)
            .finish()
    }
}
