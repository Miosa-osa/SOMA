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

/// Incremental SHA-256 over data that is never held in memory as one slice.
///
/// Capture hashes a memory object through the same open handle it wrote and synced, so the
/// recorded digest describes the published bytes rather than a separately read copy.
#[derive(Clone, Default)]
pub struct Hasher(Sha256);

impl Hasher {
    #[must_use]
    pub fn new() -> Self {
        Self(Sha256::default())
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    #[must_use]
    pub fn finish(self) -> Digest {
        let output = self.0.finalize();
        let mut digest = [0_u8; 32];
        digest.copy_from_slice(output.as_ref());
        Digest(digest)
    }
}

impl fmt::Debug for Hasher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Hasher(sha256)")
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

    #[test]
    fn the_incremental_hasher_matches_the_one_shot_digest() {
        use super::Hasher;

        let mut hasher = Hasher::new();
        hasher.update(b"soma");
        hasher.update(b"-snapshot");
        assert_eq!(hasher.finish(), Digest::of(b"soma-snapshot"));
        assert_eq!(Hasher::default().finish(), Digest::of(&[]));
        assert_eq!(format!("{:?}", Hasher::new()), "Hasher(sha256)");
    }
}
