//! Names, mount options, digests, and free-space evidence of an overlay class.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::dimensions::{DimensionError, MAX_CLASS_NAME_BYTES};

/// One accepted guest mount option for the overlay head.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MountOption {
    /// `noatime`.
    NoAtime,
    /// `nodiratime`.
    NoDirAtime,
    /// `nodev`.
    NoDev,
    /// `nosuid`.
    NoSuid,
    /// `data=ordered`.
    DataOrdered,
    /// `errors=remount-ro`.
    ErrorsRemountRo,
}

impl MountOption {
    /// The exact mount option text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoAtime => "noatime",
            Self::NoDirAtime => "nodiratime",
            Self::NoDev => "nodev",
            Self::NoSuid => "nosuid",
            Self::DataOrdered => "data=ordered",
            Self::ErrorsRemountRo => "errors=remount-ro",
        }
    }
}

/// Ordered, duplicate-free list of accepted mount options.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MountOptions(Vec<MountOption>);

impl MountOptions {
    /// Keeps the first occurrence of each option in the given order.
    #[must_use]
    pub fn new(options: &[MountOption]) -> Self {
        let mut unique = Vec::with_capacity(options.len());
        for option in options {
            if !unique.contains(option) {
                unique.push(*option);
            }
        }
        Self(unique)
    }

    /// The options in order.
    #[must_use]
    pub fn as_slice(&self) -> &[MountOption] {
        &self.0
    }

    /// Comma-joined text suitable for a mount command line.
    #[must_use]
    pub fn render(&self) -> String {
        self.0
            .iter()
            .map(|option| option.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// SHA-256 digest of the complete template bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TemplateDigest([u8; 32]);

impl TemplateDigest {
    /// Wraps raw digest bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Parses 64 lowercase hex digits.
    ///
    /// # Errors
    ///
    /// Returns [`DimensionError::DigestTextInvalid`] for any other text.
    pub fn from_hex(text: &str) -> Result<Self, DimensionError> {
        if text.len() != 64
            || !text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(DimensionError::DigestTextInvalid);
        }
        let mut bytes = [0u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
                .map_err(|_| DimensionError::DigestTextInvalid)?;
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for TemplateDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for TemplateDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TemplateDigest({self})")
    }
}

/// Validated overlay class name, `[a-z0-9-]{1,32}`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ClassName(String);

impl ClassName {
    /// Validates a class name.
    ///
    /// # Errors
    ///
    /// Returns [`DimensionError::ClassNameInvalid`] for an empty, long, or mixed name.
    pub fn new(value: impl Into<String>) -> Result<Self, DimensionError> {
        let value = value.into();
        let accepted = !value.is_empty()
            && value.len() <= MAX_CLASS_NAME_BYTES
            && value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !value.starts_with('-');
        if !accepted {
            return Err(DimensionError::ClassNameInvalid);
        }
        Ok(Self(value))
    }

    /// The validated name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ClassName {
    type Error = DimensionError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ClassName> for String {
    fn from(value: ClassName) -> Self {
        value.0
    }
}

/// Minimum free space the receiving filesystem must prove before a class is admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FreeSpaceEvidence {
    /// Free bytes required on the head filesystem at admission time.
    pub minimum_free_bytes: u64,
}
