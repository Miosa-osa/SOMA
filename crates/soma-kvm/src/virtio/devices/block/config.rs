//! Why a block device could not be constructed or given the store it serves.
//!
//! The two refusals worth naming are the ones a launch depends on: a device takes exactly one
//! store, and a durable volume is the only role whose first store is a placeholder.

use std::fmt;

/// Why a block device could not be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockConfigError {
    /// The backend's writability disagrees with the role.
    RoleMismatch,
    /// The logical block size is not a power of two in `512..=4096`.
    InvalidBlockSize { blk_size: u32 },
    /// The capacity is zero or not a multiple of the block size.
    InvalidCapacity { capacity_bytes: u64 },
    /// An attached store does not have the shape the device was built for.
    ///
    /// The guest has already been told the device's capacity, so a store of a different size
    /// cannot be substituted for the one the device was built against.
    AttachedShapeDiffers,
    /// A live store was offered to a device that already serves one.
    ///
    /// A durable volume takes exactly one swap, from the placeholder it was built on to the
    /// Instance's own file, and a second swap would replace a store a running guest may already
    /// have written to.
    AlreadyAttached,
    /// A durable volume is still backed by its placeholder where its data should be.
    ///
    /// A guest that ran on this device would read an empty disk where its files belong, so the
    /// machine refuses to run and a capture refuses to publish it.
    PlaceholderSwapMissing,
}

impl fmt::Display for BlockConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "block device rejected: {self:?}")
    }
}

impl std::error::Error for BlockConfigError {}
