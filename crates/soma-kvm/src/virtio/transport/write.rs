//! Driver-side register writes for [`MmioTransport`].

use crate::virtio::device::{VIRTIO_F_VERSION_1, VirtioDevice};
use crate::virtio::guest_memory::GuestMemory;
use crate::virtio::queue::Queue;
use crate::virtio::transport::registers::{AccessWidth, Register};
use crate::virtio::transport::status::{STATUS_DRIVER_OK, STATUS_FEATURES_OK, StatusWrite};
use crate::virtio::transport::violation::TransportViolation;
use crate::virtio::transport::{INTERRUPT_KNOWN, MmioTransport, TransportEvent};

impl<D: VirtioDevice> MmioTransport<D> {
    /// Applies a driver write to a transport register or configuration range.
    ///
    /// `mem` is needed so `QueueReady` can prove ring containment.
    ///
    /// # Errors
    /// Returns the typed violation, which is also counted.
    /// A rejected write never partially mutates transport state.
    pub fn write<M: GuestMemory + ?Sized>(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        mem: &M,
    ) -> Result<TransportEvent, TransportViolation> {
        let register =
            Register::decode(offset).ok_or(TransportViolation::UnknownRegister { offset })?;
        if let Register::Config(config_offset) = register {
            return self.write_config(config_offset, width, value);
        }
        if width != AccessWidth::U32 {
            return Err(self.record(TransportViolation::WidthMismatch { offset }));
        }
        let word = u32::try_from(value).map_err(|_| TransportViolation::WidthMismatch { offset });
        let word = word.map_err(|violation| self.record(violation))?;
        match register {
            Register::DeviceFeaturesSel => self.device_features_sel = word,
            Register::DriverFeaturesSel => self.driver_features_sel = word,
            Register::DriverFeatures => self.write_driver_features(offset, word)?,
            Register::QueueSel => self.queue_sel = word,
            Register::QueueNum => self.with_selected_queue(offset, |queue| {
                let size = u16::try_from(word).unwrap_or(u16::MAX);
                queue.set_size(size).map_err(TransportViolation::Queue)
            })?,
            Register::QueueReady => return self.write_queue_ready(offset, word, mem),
            Register::QueueNotify => return self.write_queue_notify(value),
            Register::InterruptAck => self.write_interrupt_ack(value)?,
            Register::Status => return self.write_status(value),
            Register::QueueDescLow => self.with_selected_queue(offset, |queue| {
                queue.set_desc_addr(low(queue.state().desc, word));
                Ok(())
            })?,
            Register::QueueDescHigh => self.with_selected_queue(offset, |queue| {
                queue.set_desc_addr(high(queue.state().desc, word));
                Ok(())
            })?,
            Register::QueueDriverLow => self.with_selected_queue(offset, |queue| {
                queue.set_avail_addr(low(queue.state().avail, word));
                Ok(())
            })?,
            Register::QueueDriverHigh => self.with_selected_queue(offset, |queue| {
                queue.set_avail_addr(high(queue.state().avail, word));
                Ok(())
            })?,
            Register::QueueDeviceLow => self.with_selected_queue(offset, |queue| {
                queue.set_used_addr(low(queue.state().used, word));
                Ok(())
            })?,
            Register::QueueDeviceHigh => self.with_selected_queue(offset, |queue| {
                queue.set_used_addr(high(queue.state().used, word));
                Ok(())
            })?,
            Register::ShmSel => {}
            Register::QueueReset => {
                return Err(self.record(TransportViolation::RingResetUnsupported));
            }
            _ => return Err(self.record(TransportViolation::WriteOfReadOnly { offset })),
        }
        Ok(TransportEvent::None)
    }

    fn write_driver_features(&mut self, offset: u64, word: u32) -> Result<(), TransportViolation> {
        if self.status.features_ok() {
            return Err(self.record(TransportViolation::ConfigurationLocked { offset }));
        }
        match self.driver_features_sel {
            0 => self.driver_features = (self.driver_features & !0xffff_ffff) | u64::from(word),
            1 => {
                self.driver_features =
                    (self.driver_features & 0xffff_ffff) | (u64::from(word) << 32);
            }
            _ => {}
        }
        Ok(())
    }

    /// Runs `apply` on the selected queue while queue configuration is unlocked.
    fn with_selected_queue(
        &mut self,
        offset: u64,
        apply: impl FnOnce(&mut Queue) -> Result<(), TransportViolation>,
    ) -> Result<(), TransportViolation> {
        if self.status.driver_ok() {
            return Err(self.record(TransportViolation::ConfigurationLocked { offset }));
        }
        let sel = self.queue_sel;
        let Some(queue) = usize::try_from(sel)
            .ok()
            .and_then(|index| self.queues.get_mut(index))
        else {
            return Err(self.record(TransportViolation::QueueSelOutOfRange { sel }));
        };
        apply(queue).map_err(|violation| self.record(violation))
    }

    fn write_queue_ready<M: GuestMemory + ?Sized>(
        &mut self,
        offset: u64,
        word: u32,
        mem: &M,
    ) -> Result<TransportEvent, TransportViolation> {
        if word == 0 {
            self.with_selected_queue(offset, |queue| {
                queue.deactivate();
                Ok(())
            })?;
            return Ok(TransportEvent::None);
        }
        if !self.status.features_ok() {
            return Err(self.record(TransportViolation::ConfigurationLocked { offset }));
        }
        self.with_selected_queue(offset, |queue| {
            queue.activate(mem).map_err(TransportViolation::Queue)
        })?;
        Ok(TransportEvent::None)
    }

    fn write_queue_notify(&mut self, value: u64) -> Result<TransportEvent, TransportViolation> {
        if !self.is_active() {
            return Err(self.record(TransportViolation::NotifyBeforeDriverOk));
        }
        let index = u16::try_from(value)
            .ok()
            .filter(|index| usize::from(*index) < self.queues.len())
            .ok_or(TransportViolation::NotifyOutOfRange { index: value });
        let index = index.map_err(|violation| self.record(violation))?;
        if !self.queues[usize::from(index)].is_ready() {
            return Err(self.record(TransportViolation::NotifyQueueNotReady { index }));
        }
        Ok(TransportEvent::QueueNotify(index))
    }

    fn write_interrupt_ack(&mut self, value: u64) -> Result<(), TransportViolation> {
        let acked = u32::try_from(value).unwrap_or(u32::MAX);
        // Clear exactly the acknowledged known bits in one store; the transport
        // is owned by a single device thread, so no concurrent raise can be lost.
        self.interrupt_status &= !(acked & INTERRUPT_KNOWN);
        if value & !u64::from(INTERRUPT_KNOWN) != 0 {
            return Err(self.record(TransportViolation::InterruptAckUnknownBits { value }));
        }
        Ok(())
    }

    fn write_status(&mut self, value: u64) -> Result<TransportEvent, TransportViolation> {
        let write = self
            .status
            .classify_write(value)
            .map_err(|violation| self.record(TransportViolation::Status(violation)))?;
        match write {
            StatusWrite::Reset => {
                self.reset_all();
                Ok(TransportEvent::Reset)
            }
            StatusWrite::Unchanged => Ok(TransportEvent::None),
            StatusWrite::SetBit(STATUS_FEATURES_OK) => {
                self.check_features()?;
                self.status = self.status.with(STATUS_FEATURES_OK);
                Ok(TransportEvent::None)
            }
            StatusWrite::SetBit(STATUS_DRIVER_OK) => {
                if let Err(error) = self.device.activate(self.driver_features) {
                    self.set_needs_reset();
                    return Err(self.record(TransportViolation::Activate(error)));
                }
                self.status = self.status.with(STATUS_DRIVER_OK);
                Ok(TransportEvent::DriverOk)
            }
            StatusWrite::SetBit(bit) => {
                self.status = self.status.with(bit);
                Ok(TransportEvent::None)
            }
        }
    }

    /// Rejects `FEATURES_OK` when the driver accepted bits outside the allowlist
    /// or omitted `VIRTIO_F_VERSION_1`; the status bit stays clear so the
    /// driver observes the failure on read-back.
    pub(super) fn check_features(&mut self) -> Result<(), TransportViolation> {
        let unsupported = self.driver_features & !self.device.feature_allowlist();
        let missing_version_1 = self.driver_features & VIRTIO_F_VERSION_1 == 0;
        if unsupported != 0 || missing_version_1 {
            return Err(self.record(TransportViolation::FeaturesRejected {
                unsupported,
                missing_version_1,
            }));
        }
        Ok(())
    }

    fn write_config(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
    ) -> Result<TransportEvent, TransportViolation> {
        let len = width.bytes();
        let start = self.config_range(offset, len)?;
        let raw = value.to_le_bytes();
        self.device
            .write_config(start, &raw[..len])
            .map_err(|error| self.record(TransportViolation::ConfigAccess(error)))?;
        Ok(TransportEvent::ConfigWritten { offset, len })
    }
}

const fn low(current: u64, word: u32) -> u64 {
    (current & !0xffff_ffff) | word as u64
}

const fn high(current: u64, word: u32) -> u64 {
    (current & 0xffff_ffff) | ((word as u64) << 32)
}
