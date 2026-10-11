//! Asking a running machine for more or less memory, and reading what the guest holds.
//!
//! The host only ever states a total it wants; the guest driver plugs or unplugs blocks toward
//! it, and the device checks every request. So `set_requested_memory` is a request and
//! `plugged_memory` is the answer, and the two meeting is what a resize is.

use super::SandboxMachine;
use crate::virtio::SizeError;

impl SandboxMachine {
    /// Asks the guest for `bytes` of plugged memory in total, on top of what it booted with.
    ///
    /// The configuration change is flagged on the device and the device thread is woken to raise
    /// its interrupt, so the guest driver re-reads the size and converges on it.
    ///
    /// # Errors
    ///
    /// Rejects a size that is not whole blocks or exceeds the region, and a machine whose
    /// Generation declared no memory device.
    pub fn set_requested_memory(&self, bytes: u64) -> Result<(), SizeError> {
        {
            let mut bus = self.shared.lock();
            let transport = bus.mem_mut().ok_or(SizeError::NoDevice)?;
            transport.device_mut().set_requested_size(bytes)?;
            transport.signal_config_change();
        }
        self.wake_devices();
        Ok(())
    }

    /// The memory the guest holds on top of its boot memory, as the device has confirmed it.
    #[must_use]
    pub fn plugged_memory(&self) -> u64 {
        self.shared
            .lock()
            .mem()
            .map_or(0, |mem| mem.device().plugged_size())
    }

    /// The size the host last asked for.
    #[must_use]
    pub fn requested_memory(&self) -> u64 {
        self.shared
            .lock()
            .mem()
            .map_or(0, |mem| mem.device().requested_size())
    }
}
