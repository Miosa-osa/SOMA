//! Portable identities for private disk heads.
//!
//! A head is addressed inside its capability directory by a [`HeadName`] that can never
//! escape the directory, and owned by a [`HeadToken`] that the allocator derives from the
//! Instance it serves.
//! Neither type carries a host path.

use std::fmt::{self, Write as _};

use serde::{Deserialize, Serialize};

/// Maximum accepted length of a head name in bytes.
pub const MAX_HEAD_NAME_BYTES: usize = 64;

/// Validated file name of one head inside its capability directory.
///
/// Names are lowercase ASCII letters, digits, and hyphens, start with a letter or digit, and
/// are at most [`MAX_HEAD_NAME_BYTES`] long, so they can never name a parent directory, hide
/// as a dotfile, contain a path separator, or carry a NUL byte.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct HeadName(String);

/// Why a candidate head name was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadNameError {
    /// The name was empty.
    Empty,
    /// The name exceeded [`MAX_HEAD_NAME_BYTES`].
    TooLong {
        /// Observed length in bytes.
        bytes: usize,
    },
    /// A byte outside the accepted alphabet was present.
    InvalidByte {
        /// Zero-based index of the offending byte.
        index: usize,
        /// The offending byte.
        byte: u8,
    },
    /// The first byte was a hyphen.
    LeadingHyphen,
}

impl fmt::Display for HeadNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("head name is empty"),
            Self::TooLong { bytes } => write!(f, "head name has {bytes} bytes"),
            Self::InvalidByte { index, byte } => {
                write!(f, "head name byte {index} is {byte:#04x}")
            }
            Self::LeadingHyphen => f.write_str("head name starts with a hyphen"),
        }
    }
}

impl std::error::Error for HeadNameError {}

impl HeadName {
    /// Validates a candidate name.
    ///
    /// # Errors
    ///
    /// Returns the first violated rule.
    pub fn new(value: impl Into<String>) -> Result<Self, HeadNameError> {
        let value = value.into();
        if value.is_empty() {
            return Err(HeadNameError::Empty);
        }
        if value.len() > MAX_HEAD_NAME_BYTES {
            return Err(HeadNameError::TooLong { bytes: value.len() });
        }
        for (index, byte) in value.bytes().enumerate() {
            let accepted = byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-';
            if !accepted {
                return Err(HeadNameError::InvalidByte { index, byte });
            }
        }
        if value.starts_with('-') {
            return Err(HeadNameError::LeadingHyphen);
        }
        Ok(Self(value))
    }

    /// The validated name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HeadName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for HeadName {
    type Error = HeadNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<HeadName> for String {
    fn from(value: HeadName) -> Self {
        value.0
    }
}

/// Opaque single-use ownership token for one head.
///
/// The allocator derives it from the Instance identity it serves; the storage crate only
/// requires it to be non-zero and unique for the life of the ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HeadToken([u8; 16]);

/// Why a candidate token was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadTokenError {
    /// Every byte was zero.
    AllZero,
}

impl fmt::Display for HeadTokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllZero => f.write_str("head token is all zero"),
        }
    }
}

impl std::error::Error for HeadTokenError {}

impl HeadToken {
    /// Accepts any non-zero 16-byte value.
    ///
    /// # Errors
    ///
    /// Returns [`HeadTokenError::AllZero`] for the all-zero value.
    pub fn new(bytes: [u8; 16]) -> Result<Self, HeadTokenError> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(HeadTokenError::AllZero);
        }
        Ok(Self(bytes))
    }

    /// The exact token bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// The canonical head name for this token, `head-` followed by 32 lowercase hex digits.
    #[must_use]
    pub fn head_name(&self) -> HeadName {
        let mut name = String::with_capacity(5 + 32);
        name.push_str("head-");
        for byte in self.0 {
            let _ = write!(name, "{byte:02x}");
        }
        HeadName(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_lowercase_alphanumeric_names_with_hyphens() {
        let name = HeadName::new("head-00ff").expect("valid");
        assert_eq!(name.as_str(), "head-00ff");
        assert_eq!(name.to_string(), "head-00ff");
    }

    #[test]
    fn rejects_empty_long_dotted_uppercase_and_separator_names() {
        assert_eq!(HeadName::new(""), Err(HeadNameError::Empty));
        let long = "a".repeat(MAX_HEAD_NAME_BYTES + 1);
        assert_eq!(
            HeadName::new(long),
            Err(HeadNameError::TooLong { bytes: 65 })
        );
        assert_eq!(
            HeadName::new(".."),
            Err(HeadNameError::InvalidByte {
                index: 0,
                byte: b'.'
            })
        );
        assert_eq!(
            HeadName::new("a/b"),
            Err(HeadNameError::InvalidByte {
                index: 1,
                byte: b'/'
            })
        );
        assert_eq!(
            HeadName::new("Head"),
            Err(HeadNameError::InvalidByte {
                index: 0,
                byte: b'H'
            })
        );
        assert_eq!(
            HeadName::new("a\0b"),
            Err(HeadNameError::InvalidByte { index: 1, byte: 0 })
        );
        assert_eq!(HeadName::new("-a"), Err(HeadNameError::LeadingHyphen));
    }

    #[test]
    fn serde_round_trip_revalidates() {
        let json = serde_json::to_string(&HeadName::new("abc").expect("valid")).expect("json");
        assert_eq!(json, "\"abc\"");
        let parsed: Result<HeadName, _> = serde_json::from_str("\"../x\"");
        assert!(parsed.is_err());
    }

    #[test]
    fn tokens_reject_zero_and_derive_stable_names() {
        assert_eq!(HeadToken::new([0; 16]), Err(HeadTokenError::AllZero));
        let mut bytes = [0u8; 16];
        bytes[0] = 0xab;
        bytes[15] = 0x01;
        let token = HeadToken::new(bytes).expect("valid");
        assert_eq!(
            token.head_name().as_str(),
            "head-ab000000000000000000000000000001"
        );
        assert_eq!(token.head_name(), token.head_name());
        assert_eq!(token.as_bytes(), &bytes);
    }
}
