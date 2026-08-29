//! Typed SHA-256 digest used for artifact identity and section integrity.

use std::fmt;

use sha2::{Digest as _, Sha256};

/// One SHA-256 digest with an exact 32-byte representation.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct Digest([u8; 32]);

impl Digest {
    pub const LEN: usize = 32;

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Hashes `bytes` completely in memory.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        let output = Sha256::digest(bytes);
        let mut digest = [0_u8; 32];
        digest.copy_from_slice(output.as_ref());
        Self(digest)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|byte| *byte == 0)
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "sha256:{self}")
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Digest;

    #[test]
    fn hashes_the_empty_input_to_the_known_sha256_value() {
        let digest = Digest::of(&[]);
        assert_eq!(
            digest.to_string(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert!(!digest.is_zero());
        assert!(Digest::from_bytes([0; 32]).is_zero());
        assert_eq!(format!("{digest:?}").len(), 7 + 64);
    }
}
