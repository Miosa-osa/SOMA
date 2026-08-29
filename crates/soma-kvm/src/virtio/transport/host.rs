//! Host-side (device thread) operations on [`MmioTransport`]: completing used
//! chains, raising interrupt status, and requesting a device reset.

use crate::virtio::device::VirtioDevice;
use crate::virtio::guest_memory::GuestMemory;
use crate::virtio::queue::chain::DescriptorChain;
use crate::virtio::transport::status::STATUS_DEVICE_NEEDS_RESET;
use crate::virtio::transport::violation::TransportViolation;
use crate::virtio::transport::{INTERRUPT_CONFIG_CHANGE, INTERRUPT_USED_BUFFER, MmioTransport};

impl<D: VirtioDevice> MmioTransport<D> {
    /// Publishes a used chain, raises the used-buffer status, and reports
    /// whether the caller must signal the device interrupt.
    ///
    /// # Errors
    /// Rejects work while inactive and forwards queue violations.
    pub fn complete_used<M: GuestMemory + ?Sized>(
        &mut self,
        index: u16,
        mem: &M,
        chain: &DescriptorChain,
        len: u32,
    ) -> Result<bool, TransportViolation> {
        if !self.is_active() {
            return Err(self.record(TransportViolation::NotifyBeforeDriverOk));
        }
        let Some(queue) = self.queues.get_mut(usize::from(index)) else {
            return Err(self.record(TransportViolation::NotifyOutOfRange {
                index: u64::from(index),
            }));
        };
        let result = queue
            .add_used(mem, chain, len)
            .and_then(|()| queue.needs_notification(mem));
        let notify = result.map_err(|violation| self.record(violation.into()))?;
        self.interrupt_status |= INTERRUPT_USED_BUFFER;
        Ok(notify)
    }

    /// Bumps the configuration generation and raises the config-change status.
    pub const fn signal_config_change(&mut self) {
        self.config_generation = self.config_generation.wrapping_add(1);
        self.interrupt_status |= INTERRUPT_CONFIG_CHANGE;
    }

    /// Marks the device as needing a reset and stops all queue work.
    pub fn set_needs_reset(&mut self) {
        self.status = self.status.with(STATUS_DEVICE_NEEDS_RESET);
        for queue in &mut self.queues {
            queue.deactivate();
        }
    }
}
