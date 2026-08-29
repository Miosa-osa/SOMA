//! Memory-object descriptor and validation for `memory.raw`.
//!
//! The descriptor binds the SHA-256 digest and the exact page-aligned byte size of the
//! immutable memory artifact.
//! Validation helpers fail closed on any size, alignment, or digest disagreement.

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[allow(unsafe_code)]
mod mapping;

use std::{error::Error, fmt};

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub use mapping::{MappingError, MappingOperation, PrivateMapping};

use super::{
    Digest, WireError,
    wire::{Reader, Writer},
};

/// Upper bound on a v1 memory object: 3 GiB plus nothing, matching the machine contract.
pub const MAX_MEMORY_BYTES: u64 = 3 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryError {
    Wire(WireError),
    ZeroSize,
    SizeExceedsBound { size: u64, bound: u64 },
    NotPageAligned { size: u64, page_size: u32 },
    ZeroDigest,
    SizeMismatch { expected: u64, actual: u64 },
    DigestMismatch { expected: Digest, actual: Digest },
}

impl fmt::Display for MemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(error) => write!(formatter, "memory descriptor wire error: {error}"),
            Self::ZeroSize => formatter.write_str("memory size must be non-zero"),
            Self::SizeExceedsBound { size, bound } => {
                write!(formatter, "memory size {size} exceeds bound {bound}")
            }
            Self::NotPageAligned { size, page_size } => {
                write!(
                    formatter,
                    "memory size {size} is not a multiple of {page_size}"
                )
            }
            Self::ZeroDigest => formatter.write_str("memory digest cannot be all zero"),
            Self::SizeMismatch { expected, actual } => {
                write!(
                    formatter,
                    "memory object is {actual} bytes, expected {expected}"
                )
            }
            Self::DigestMismatch { expected, actual } => {
                write!(
                    formatter,
                    "memory digest {actual:?} does not match {expected:?}"
                )
            }
        }
    }
}

impl Error for MemoryError {}

impl From<WireError> for MemoryError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

/// Identity and exact size of one immutable memory object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryDescriptor {
    digest: Digest,
    size: u64,
}

impl MemoryDescriptor {
    /// Creates a descriptor whose size is non-zero, bounded, and a multiple of `page_size`.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError::ZeroSize`], [`MemoryError::SizeExceedsBound`],
    /// [`MemoryError::NotPageAligned`], or [`MemoryError::ZeroDigest`].
    pub fn new(digest: Digest, size: u64, page_size: u32) -> Result<Self, MemoryError> {
        if size == 0 {
            return Err(MemoryError::ZeroSize);
        }
        if size > MAX_MEMORY_BYTES {
            return Err(MemoryError::SizeExceedsBound {
                size,
                bound: MAX_MEMORY_BYTES,
            });
        }
        if page_size == 0 || !size.is_multiple_of(u64::from(page_size)) {
            return Err(MemoryError::NotPageAligned { size, page_size });
        }
        if digest.is_zero() {
            return Err(MemoryError::ZeroDigest);
        }
        Ok(Self { digest, size })
    }

    #[must_use]
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Checks that an opened memory object has exactly the certified length.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError::SizeMismatch`] on any difference.
    pub const fn verify_length(&self, actual: u64) -> Result<(), MemoryError> {
        if actual == self.size {
            Ok(())
        } else {
            Err(MemoryError::SizeMismatch {
                expected: self.size,
                actual,
            })
        }
    }

    /// Checks that the Generation manifest and this descriptor agree exactly.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError::SizeMismatch`] or [`MemoryError::DigestMismatch`].
    pub fn verify_generation(&self, digest: Digest, size: u64) -> Result<(), MemoryError> {
        self.verify_length(size)?;
        if digest == self.digest {
            Ok(())
        } else {
            Err(MemoryError::DigestMismatch {
                expected: self.digest,
                actual: digest,
            })
        }
    }

    /// Hashes a complete in-memory image and compares length and digest.
    ///
    /// This is an installation or audit operation and is never run on the Launch path.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError::SizeMismatch`] or [`MemoryError::DigestMismatch`].
    pub fn verify_bytes(&self, bytes: &[u8]) -> Result<(), MemoryError> {
        self.verify_length(u64::try_from(bytes.len()).unwrap_or(u64::MAX))?;
        let actual = Digest::of(bytes);
        if actual == self.digest {
            Ok(())
        } else {
            Err(MemoryError::DigestMismatch {
                expected: self.digest,
                actual,
            })
        }
    }

    pub(crate) fn encode(&self, writer: &mut Writer) {
        writer.put_bytes(self.digest.as_bytes());
        writer.put_u64(self.size);
    }

    pub(crate) fn decode(reader: &mut Reader<'_>, page_size: u32) -> Result<Self, MemoryError> {
        let digest = Digest::from_bytes(reader.array()?);
        let size = reader.u64()?;
        Self::new(digest, size, page_size)
    }
}

#[cfg(test)]
mod tests {
    use super::{Digest, MAX_MEMORY_BYTES, MemoryDescriptor, MemoryError, Reader, Writer};

    fn digest() -> Digest {
        Digest::of(b"memory")
    }

    #[test]
    fn rejects_zero_unaligned_oversized_and_zero_digest() {
        assert_eq!(
            MemoryDescriptor::new(digest(), 0, 4096),
            Err(MemoryError::ZeroSize)
        );
        assert_eq!(
            MemoryDescriptor::new(digest(), 4097, 4096),
            Err(MemoryError::NotPageAligned {
                size: 4097,
                page_size: 4096
            })
        );
        assert_eq!(
            MemoryDescriptor::new(digest(), 4096, 0),
            Err(MemoryError::NotPageAligned {
                size: 4096,
                page_size: 0
            })
        );
        assert_eq!(
            MemoryDescriptor::new(digest(), MAX_MEMORY_BYTES + 4096, 4096),
            Err(MemoryError::SizeExceedsBound {
                size: MAX_MEMORY_BYTES + 4096,
                bound: MAX_MEMORY_BYTES
            })
        );
        assert_eq!(
            MemoryDescriptor::new(Digest::from_bytes([0; 32]), 4096, 4096),
            Err(MemoryError::ZeroDigest)
        );
    }

    #[test]
    fn verifies_exact_size_and_digest() {
        let bytes = vec![7_u8; 8192];
        let descriptor = MemoryDescriptor::new(Digest::of(&bytes), 8192, 4096).unwrap();
        assert_eq!(descriptor.verify_bytes(&bytes), Ok(()));
        assert_eq!(
            descriptor.verify_bytes(&bytes[..4096]),
            Err(MemoryError::SizeMismatch {
                expected: 8192,
                actual: 4096
            })
        );
        let mut altered = bytes;
        altered[0] = 8;
        assert!(matches!(
            descriptor.verify_bytes(&altered),
            Err(MemoryError::DigestMismatch { .. })
        ));
        assert_eq!(
            descriptor.verify_generation(descriptor.digest(), 8192),
            Ok(())
        );
        assert!(matches!(
            descriptor.verify_generation(digest(), 8192),
            Err(MemoryError::DigestMismatch { .. })
        ));
    }

    #[test]
    fn round_trips_through_the_wire() {
        let descriptor = MemoryDescriptor::new(digest(), 1 << 20, 4096).unwrap();
        let mut writer = Writer::default();
        descriptor.encode(&mut writer);
        let bytes = writer.finish();
        assert_eq!(bytes.len(), 40);
        let mut reader = Reader::new(&bytes);
        assert_eq!(MemoryDescriptor::decode(&mut reader, 4096), Ok(descriptor));
        assert!(matches!(
            MemoryDescriptor::decode(&mut Reader::new(&bytes), 1 << 21),
            Err(MemoryError::NotPageAligned { .. })
        ));
        assert!(matches!(
            MemoryDescriptor::decode(&mut Reader::new(&bytes[..10]), 4096),
            Err(MemoryError::Wire(_))
        ));
    }
}
