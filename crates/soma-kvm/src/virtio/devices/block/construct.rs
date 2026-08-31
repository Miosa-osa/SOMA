//! Building one block device and giving it the store it serves.
//!
//! A device is built against a store's shape rather than against a particular store, because a
//! prepared worker builds its overlay slot before it is allowed to hold the private head that
//! slot will serve. Construction therefore fixes the geometry the guest is told, and attachment
//! puts a store of exactly that geometry behind it.

use super::{
    BLOCK_SERIAL_LEN, BlockBackend, BlockConfigError, BlockCounters, BlockDevice, BlockRole,
    SECTOR_SIZE,
};

impl BlockDevice {
    /// Binds a backend to a role.
    ///
    /// # Errors
    /// Rejects a role/backend mismatch, a bad block size, or a bad capacity.
    pub fn new(
        role: BlockRole,
        backend: Box<dyn BlockBackend + Send>,
        blk_size: u32,
        serial: [u8; BLOCK_SERIAL_LEN],
    ) -> Result<Self, BlockConfigError> {
        if backend.read_only() != role.read_only() {
            return Err(BlockConfigError::RoleMismatch);
        }
        if !blk_size.is_power_of_two() || !(512..=4096).contains(&blk_size) {
            return Err(BlockConfigError::InvalidBlockSize { blk_size });
        }
        let capacity_bytes = backend.capacity_bytes();
        if capacity_bytes == 0 || !capacity_bytes.is_multiple_of(u64::from(blk_size)) {
            return Err(BlockConfigError::InvalidCapacity { capacity_bytes });
        }
        Ok(Self {
            role,
            backend,
            blk_size,
            capacity_sectors: capacity_bytes / SECTOR_SIZE,
            serial,
            activated: false,
            counters: BlockCounters::default(),
        })
    }
}
