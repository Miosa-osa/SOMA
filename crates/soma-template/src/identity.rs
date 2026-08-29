//! `LockId`: the SHA-256 identity of canonical Template Lock bytes.

use std::{error::Error, fmt};

use sha2::{Digest as _, Sha256};

use crate::resolve::hex;

/// The content identity of one Template Lock: `sha256:` plus the digest of its bytes.
///
/// Two locks with the same identity selected exactly the same inputs.
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct LockId([u8; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LockIdError;

impl fmt::Display for LockIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("lock identity must be sha256: plus 64 lowercase hex digits")
    }
}

impl Error for LockIdError {}

impl LockId {
    /// Computes the identity of canonical lock bytes.
    #[must_use]
    pub fn of(lock_bytes: &[u8]) -> Self {
        Self(Sha256::digest(lock_bytes).into())
    }

    /// Parses the `sha256:<hex>` form.
    ///
    /// # Errors
    ///
    /// Returns [`LockIdError`] unless the value is `sha256:` plus 64 lowercase hex digits.
    pub fn parse(value: &str) -> Result<Self, LockIdError> {
        let hex = value.strip_prefix("sha256:").ok_or(LockIdError)?;
        if hex.len() != 64 {
            return Err(LockIdError);
        }
        let mut bytes = [0_u8; 32];
        for (index, pair) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            bytes[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
        }
        Ok(Self(bytes))
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

const fn nibble(value: u8) -> Result<u8, LockIdError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(LockIdError),
    }
}

impl fmt::Display for LockId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "sha256:{}", hex(&self.0))
    }
}

impl fmt::Debug for LockId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("LockId")
            .field(&self.to_string())
            .finish()
    }
}
