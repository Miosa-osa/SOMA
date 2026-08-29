//! Validated dimensions of an overlay class.
//!
//! Every value here is a technical property of the sterile ext4 template or of the mount that
//! will receive it; product tiers, quotas, and billing stay outside this crate.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Smallest logical size accepted for an overlay class, 16 MiB.
pub const MIN_LOGICAL_BYTES: u64 = 16 * 1024 * 1024;
/// Largest logical size accepted for an overlay class, 1 TiB.
pub const MAX_LOGICAL_BYTES: u64 = 1024 * 1024 * 1024 * 1024;
/// Maximum accepted length of a class name in bytes.
pub const MAX_CLASS_NAME_BYTES: usize = 32;

/// ext4 block size of the template.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BlockSize {
    /// 1024-byte blocks.
    B1024,
    /// 2048-byte blocks.
    B2048,
    /// 4096-byte blocks, the only size the first profile certifies.
    B4096,
}

impl BlockSize {
    /// The block size in bytes.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        match self {
            Self::B1024 => 1024,
            Self::B2048 => 2048,
            Self::B4096 => 4096,
        }
    }
}

/// Logical size of the template and of every head cloned from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LogicalBytes(u64);

/// Why a dimension value was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DimensionError {
    /// The logical size is outside `[MIN_LOGICAL_BYTES, MAX_LOGICAL_BYTES]`.
    LogicalBytesOutOfRange {
        /// Requested size.
        bytes: u64,
    },
    /// The logical size is not a multiple of the block size.
    LogicalBytesUnaligned {
        /// Requested size.
        bytes: u64,
        /// Block size in bytes.
        block: u64,
    },
    /// The bytes-per-inode ratio is not a power of two within `[1024, 64 MiB]`.
    InodeRatioInvalid {
        /// Requested ratio.
        bytes_per_inode: u32,
    },
    /// The class name is empty, too long, or uses a byte outside `[a-z0-9-]`.
    ClassNameInvalid,
    /// The hexadecimal digest text is not exactly 64 lowercase hex digits.
    DigestTextInvalid,
}

impl fmt::Display for DimensionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LogicalBytesOutOfRange { bytes } => write!(f, "{bytes} bytes is out of range"),
            Self::LogicalBytesUnaligned { bytes, block } => {
                write!(f, "{bytes} bytes is not a multiple of {block}")
            }
            Self::InodeRatioInvalid { bytes_per_inode } => {
                write!(f, "{bytes_per_inode} bytes per inode is not accepted")
            }
            Self::ClassNameInvalid => f.write_str("class name is invalid"),
            Self::DigestTextInvalid => f.write_str("digest text is invalid"),
        }
    }
}

impl std::error::Error for DimensionError {}

impl LogicalBytes {
    /// Accepts a size within range that is a multiple of `block`.
    ///
    /// # Errors
    ///
    /// Returns the violated bound.
    pub fn new(bytes: u64, block: BlockSize) -> Result<Self, DimensionError> {
        if !(MIN_LOGICAL_BYTES..=MAX_LOGICAL_BYTES).contains(&bytes) {
            return Err(DimensionError::LogicalBytesOutOfRange { bytes });
        }
        if !bytes.is_multiple_of(block.bytes()) {
            return Err(DimensionError::LogicalBytesUnaligned {
                bytes,
                block: block.bytes(),
            });
        }
        Ok(Self(bytes))
    }

    /// The exact size in bytes.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// How the ext4 filesystem UUID of a class is chosen.
///
/// Launch never runs `tune2fs`, so every head of one class shares the template UUID; the guest
/// mounts the overlay by device rather than by UUID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UuidPolicy {
    /// The UUID is derived from the class name, version, and logical size.
    Derived,
    /// An operator-supplied fixed UUID.
    Explicit([u8; 16]),
}

/// Pinned ext4 feature set written by `mke2fs -O`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Ext4FeatureSet {
    /// Journaled, 64-bit, extent-mapped, checksummed ext4 without orphan files or a random
    /// checksum seed.
    V1,
}

impl Ext4FeatureSet {
    /// The exact `-O` argument, starting with `none` so `mke2fs.conf` defaults cannot leak in.
    #[must_use]
    pub const fn mke2fs_argument(self) -> &'static str {
        match self {
            Self::V1 => {
                "none,has_journal,ext_attr,resize_inode,dir_index,filetype,extent,flex_bg,\
                 sparse_super,large_file,huge_file,dir_nlink,extra_isize,metadata_csum,64bit"
            }
        }
    }
}

/// How many inodes the template carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InodePolicy {
    /// One inode per this many bytes, passed as `mke2fs -i`.
    BytesPerInode(u32),
}

impl InodePolicy {
    /// Accepts a power-of-two ratio between 1 KiB and 64 MiB.
    ///
    /// # Errors
    ///
    /// Returns [`DimensionError::InodeRatioInvalid`] otherwise.
    pub fn bytes_per_inode(bytes_per_inode: u32) -> Result<Self, DimensionError> {
        let accepted = bytes_per_inode.is_power_of_two()
            && (1024..=64 * 1024 * 1024).contains(&bytes_per_inode);
        if !accepted {
            return Err(DimensionError::InodeRatioInvalid { bytes_per_inode });
        }
        Ok(Self::BytesPerInode(bytes_per_inode))
    }
}
