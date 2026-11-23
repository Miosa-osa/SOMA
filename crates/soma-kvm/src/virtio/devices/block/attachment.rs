//! Putting the real store behind a device that was built against a declared shape.
//!
//! A prepared worker builds its overlay device before it is allowed to hold the private head
//! that device will serve, so the head arrives afterwards and this is where it lands. The
//! replacement is checked against the shape the guest was already told, because a device that
//! changed its own capacity underneath a running guest would be lying about itself.

use super::{BlockBackend, BlockConfigError, BlockDevice, BlockRole, SECTOR_SIZE};

impl BlockDevice {
    /// Flushes and relinquishes the durable volume while retaining its device geometry.
    #[cfg(any(all(test, unix), all(target_os = "linux", target_arch = "x86_64")))]
    pub(crate) fn release_volume(&mut self) -> Result<(), super::backend::BackendError> {
        if self.role != BlockRole::DurableVolume {
            return Err(super::backend::BackendError::OutOfRange);
        }
        if !self.placeholder {
            self.backend.flush()?;
            self.backend = Box::new(super::backend::Detached::new(
                self.capacity_sectors * SECTOR_SIZE,
                false,
            ));
            self.placeholder = true;
        }
        Ok(())
    }

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
        // Attaching cannot displace a volume the guest may have written to. A separately
        // confirmed release must return the slot to its placeholder first.
        if self.role == BlockRole::DurableVolume && !self.placeholder {
            return Err(BlockConfigError::AlreadyAttached);
        }
        self.backend = backend;
        self.placeholder = false;
        Ok(())
    }
}
