//! Fixed-width identities that cross the allocator seam.
//!
//! The allocator owns its own identities so that the ledger, the protocol, and the pool key
//! have one exact encoding.
//! Every identity rejects the all-zero value and exposes its bytes for ledger comparison.

use std::fmt;

use sha2::{Digest, Sha256};

/// Why an identity or generation was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdError {
    /// The value was all zero.
    Zero(&'static str),
    /// A lease generation would exceed the packed range.
    GenerationExhausted,
}

impl fmt::Display for IdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zero(label) => write!(formatter, "{label} identity is all zero"),
            Self::GenerationExhausted => formatter.write_str("lease generation exhausted"),
        }
    }
}

impl std::error::Error for IdError {}

macro_rules! fixed_id {
    ($name:ident, $label:literal, $len:literal) => {
        #[doc = concat!("A validated ", stringify!($len), "-byte ", $label, " identity.")]
        #[derive(Clone, Copy, Eq, Hash, PartialEq, PartialOrd, Ord)]
        pub struct $name([u8; $len]);

        impl $name {
            /// Validates one identity.
            ///
            /// # Errors
            ///
            /// Returns [`IdError::Zero`] for the all-zero value.
            pub fn new(bytes: [u8; $len]) -> Result<Self, IdError> {
                if bytes.iter().all(|byte| *byte == 0) {
                    Err(IdError::Zero($label))
                } else {
                    Ok(Self(bytes))
                }
            }

            /// Returns the exact bytes.
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; $len] {
                &self.0
            }

            /// Returns the identity as lowercase hexadecimal.
            #[must_use]
            pub fn hex(&self) -> String {
                self.0.iter().map(|byte| format!("{byte:02x}")).collect()
            }

            /// Returns the first four bytes as eight hex characters.
            #[must_use]
            pub fn short_hex(&self) -> String {
                self.0[..4]
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}({}..)", $label, self.short_hex())
            }
        }
    };
}

fixed_id!(WorkerId, "worker", 16);
fixed_id!(InstanceId, "instance", 16);
fixed_id!(OperationId, "operation", 16);
fixed_id!(GenerationId, "generation", 32);
fixed_id!(HostProfileDigest, "host-profile", 32);
fixed_id!(RequestFingerprint, "fingerprint", 32);
fixed_id!(LaunchMaterialHandle, "launch-material", 32);

impl RequestFingerprint {
    /// Digests the exact bytes of one request into a fingerprint.
    ///
    /// SHA-256 has no known all-zero output, so the digest is accepted directly.
    #[must_use]
    pub fn of(request: &[u8]) -> Self {
        Self(Sha256::digest(request).into())
    }
}

/// The monotonically increasing lease generation of one worker.
///
/// A claim bumps the generation, so a stale handle from an earlier lease can never act on a
/// later one; the value is nonzero and fits the 56-bit packed state word.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct LeaseGeneration(u64);

impl LeaseGeneration {
    /// The generation of a freshly constructed worker.
    pub const FIRST: Self = Self(1);
    /// The largest packable generation.
    pub const MAX: Self = Self((1 << 56) - 1);

    /// Validates one generation.
    ///
    /// # Errors
    ///
    /// Returns [`IdError::Zero`] for zero and [`IdError::GenerationExhausted`] above the
    /// packed range.
    pub fn new(value: u64) -> Result<Self, IdError> {
        if value == 0 {
            Err(IdError::Zero("lease-generation"))
        } else if value > Self::MAX.0 {
            Err(IdError::GenerationExhausted)
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the raw value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the next generation.
    ///
    /// # Errors
    ///
    /// Returns [`IdError::GenerationExhausted`] at the packed limit.
    pub fn next(self) -> Result<Self, IdError> {
        Self::new(self.0.checked_add(1).ok_or(IdError::GenerationExhausted)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_reject_zero_and_expose_bytes() {
        assert_eq!(WorkerId::new([0; 16]), Err(IdError::Zero("worker")));
        assert_eq!(GenerationId::new([0; 32]), Err(IdError::Zero("generation")));
        let id =
            WorkerId::new([0xab, 0xcd, 1, 2, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 1]).expect("nonzero");
        assert_eq!(id.short_hex(), "abcd0102");
        assert_eq!(id.hex().len(), 32);
        assert_eq!(format!("{id:?}"), "worker(abcd0102..)");
    }

    #[test]
    fn fingerprints_are_deterministic_and_input_sensitive() {
        assert_eq!(RequestFingerprint::of(b"a"), RequestFingerprint::of(b"a"));
        assert_ne!(RequestFingerprint::of(b"a"), RequestFingerprint::of(b"b"));
    }

    #[test]
    fn lease_generations_are_nonzero_monotonic_and_bounded() {
        assert_eq!(
            LeaseGeneration::new(0),
            Err(IdError::Zero("lease-generation"))
        );
        assert_eq!(LeaseGeneration::FIRST.next().expect("next").get(), 2);
        assert_eq!(
            LeaseGeneration::MAX.next(),
            Err(IdError::GenerationExhausted)
        );
        assert_eq!(
            LeaseGeneration::new(1 << 56),
            Err(IdError::GenerationExhausted)
        );
    }
}
