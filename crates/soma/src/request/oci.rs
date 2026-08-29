use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use super::ValidationError;

#[derive(Clone, PartialEq, Eq)]
pub struct OciImage(String);

impl OciImage {
    /// Parses a bounded OCI image reference without resolving it.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidImageReference`] for an empty, oversized, URL-like,
    /// whitespace-bearing, escaped, or option-like value.
    pub fn parse(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 1_024
            && !value.contains("://")
            && !value.contains(char::is_whitespace)
            && !value.contains(['\0', '\\'])
            && !value.starts_with('-');
        if !valid {
            return Err(ValidationError::InvalidImageReference);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for OciImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OciImage([REDACTED])")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct OciDigest(String);

impl OciDigest {
    /// Parses a canonical SHA-256 OCI digest.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidDigest`] unless the value has the exact lowercase
    /// `sha256:` digest form.
    pub fn parse(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(ValidationError::InvalidDigest);
        };
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ValidationError::InvalidDigest);
        }
        Ok(Self(format!("sha256:{}", hex.to_ascii_lowercase())))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for OciDigest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl fmt::Debug for OciDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("OciDigest").field(&self.0).finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OciPlatform {
    operating_system: String,
    architecture: String,
    variant: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OciPlatformWire {
    operating_system: String,
    architecture: String,
    variant: Option<String>,
}

impl<'de> Deserialize<'de> for OciPlatform {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = OciPlatformWire::deserialize(deserializer)?;
        Self::new(wire.operating_system, wire.architecture, wire.variant).map_err(D::Error::custom)
    }
}

impl OciPlatform {
    /// Creates a bounded canonical OCI platform identity.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidPlatform`] when any component is empty, oversized, or
    /// contains characters outside the portable lowercase platform grammar.
    pub fn new(
        operating_system: impl Into<String>,
        architecture: impl Into<String>,
        variant: Option<String>,
    ) -> Result<Self, ValidationError> {
        let operating_system = operating_system.into();
        let architecture = architecture.into();
        if !valid_platform_part(&operating_system)
            || !valid_platform_part(&architecture)
            || variant
                .as_deref()
                .is_some_and(|value| !valid_platform_part(value))
        {
            return Err(ValidationError::InvalidPlatform);
        }
        Ok(Self {
            operating_system,
            architecture,
            variant,
        })
    }

    #[must_use]
    pub fn linux_arm64() -> Self {
        Self {
            operating_system: "linux".to_owned(),
            architecture: "arm64".to_owned(),
            variant: None,
        }
    }

    #[must_use]
    pub fn linux_amd64() -> Self {
        Self {
            operating_system: "linux".to_owned(),
            architecture: "amd64".to_owned(),
            variant: None,
        }
    }

    #[must_use]
    pub fn operating_system(&self) -> &str {
        &self.operating_system
    }

    #[must_use]
    pub fn architecture(&self) -> &str {
        &self.architecture
    }

    #[must_use]
    pub fn variant(&self) -> Option<&str> {
        self.variant.as_deref()
    }
}

fn valid_platform_part(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
}
