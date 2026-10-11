//! Where an unplugged block goes back to.

use std::io;

/// The host memory behind the region.
pub trait RegionBacking {
    /// Makes `[offset, offset + len)` of the region usable by the guest, and by device buffers
    /// the guest places there.
    ///
    /// Called before the device marks the blocks plugged, so a block the host cannot back is
    /// refused rather than promised. Offsets are relative to the start of the region.
    ///
    /// # Errors
    /// Returns the host failure; the blocks then stay unplugged.
    fn plug(&mut self, offset: u64, len: u64) -> io::Result<()>;

    /// Returns `[offset, offset + len)` of the region to the host, so it reads as zeros and costs
    /// nothing until it is touched again.
    ///
    /// # Errors
    /// Returns the host failure; the block then stays plugged.
    fn discard(&mut self, offset: u64, len: u64) -> io::Result<()>;
}

/// A backing that records what was discarded, for tests and for a region with no host mapping yet.
#[derive(Debug, Default)]
pub struct RecordingBacking {
    /// Every range plugged, in order.
    pub plugged: std::sync::Arc<std::sync::Mutex<Vec<(u64, u64)>>>,
    /// Every range discarded, in order.
    pub discarded: std::sync::Arc<std::sync::Mutex<Vec<(u64, u64)>>>,
    /// Makes the next discard fail.
    pub fail: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl RegionBacking for RecordingBacking {
    fn plug(&mut self, offset: u64, len: u64) -> io::Result<()> {
        if self.fail.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(io::Error::other("plug failed"));
        }
        self.plugged
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((offset, len));
        Ok(())
    }

    fn discard(&mut self, offset: u64, len: u64) -> io::Result<()> {
        if self.fail.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(io::Error::other("discard failed"));
        }
        self.discarded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((offset, len));
        Ok(())
    }
}
