//! Putting the real store behind a device that was built against a declared shape.
//!
//! A prepared worker builds its overlay device before it is allowed to hold the private head
//! that device will serve, so the head arrives afterwards and this is where it lands. The
//! replacement is checked against the shape the guest was already told, because a device that
//! changed its own capacity underneath a running guest would be lying about itself.

use super::{BlockBackend, BlockConfigError, BlockDevice, BlockRole, SECTOR_SIZE};

impl BlockDevice {
    /// Puts the real store behind a device that was built against a declared shape.
    ///
    /// A prepared worker builds this device before it may hold a private disk head, so the head
    /// arrives afterwards and this is where it lands. The replacement must have exactly the shape
    /// the device was built with: the guest has been told a capacity and a writability, and a
    /// store that disagrees with either would make the device lie about itself. Nothing else
    /// about the device changes, so a restored device stays restored across the attachment.
    ///
    /// # Errors
    ///
    /// Returns [`BlockConfigError::AttachedShapeDiffers`] when the store's capacity or
    /// writability is not the one the device was built for.
    pub fn attach(
        &mut self,
        backend: Box<dyn BlockBackend + Send>,
    ) -> Result<(), BlockConfigError> {
        if backend.read_only() != self.role.read_only() {
            return Err(BlockConfigError::RoleMismatch);
        }
        if backend.capacity_bytes() != self.capacity_sectors * SECTOR_SIZE {
            return Err(BlockConfigError::AttachedShapeDiffers);
        }
        // A durable volume takes exactly one swap. A second one would replace a store a guest
        // may already have written to, and a volume that is no longer on its placeholder was not
        // built by this path, so neither swap is one this device may make.
        if self.role == BlockRole::DurableVolume && !self.placeholder {
            return Err(BlockConfigError::AlreadyAttached);
        }
        self.backend = backend;
        self.placeholder = false;
        Ok(())
    }
}
