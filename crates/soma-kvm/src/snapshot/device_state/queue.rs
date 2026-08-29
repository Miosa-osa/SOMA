//! Split-virtqueue geometry and cursors.

use super::DeviceStateError;
use crate::snapshot::wire::{Reader, Writer};

/// The vsock device has the most queues in device contract v1.
pub const MAX_QUEUES: usize = 3;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct QueueState {
    /// Device maximum, a power of two.
    pub max_size: u16,
    /// Driver-selected size: zero, or a power of two no larger than `max_size`.
    pub size: u16,
    pub ready: bool,
    pub descriptor_address: u64,
    pub available_address: u64,
    pub used_address: u64,
    pub next_available: u16,
    pub next_used: u16,
}

impl QueueState {
    pub const ENCODED_LEN: usize = 2 + 2 + 1 + 8 + 8 + 8 + 2 + 2;

    /// # Errors
    ///
    /// Returns [`DeviceStateError::InvalidQueue`] when sizes or readiness are inconsistent.
    pub fn validate(&self, index: usize) -> Result<(), DeviceStateError> {
        let invalid = |field| DeviceStateError::InvalidQueue { index, field };
        if !self.max_size.is_power_of_two() {
            return Err(invalid("max_size"));
        }
        if self.size > self.max_size || (self.size != 0 && !self.size.is_power_of_two()) {
            return Err(invalid("size"));
        }
        if self.ready && self.size == 0 {
            return Err(invalid("ready"));
        }
        Ok(())
    }

    pub(super) fn write(&self, writer: &mut Writer) {
        writer.put_u16(self.max_size);
        writer.put_u16(self.size);
        writer.put_presence(self.ready);
        writer.put_u64(self.descriptor_address);
        writer.put_u64(self.available_address);
        writer.put_u64(self.used_address);
        writer.put_u16(self.next_available);
        writer.put_u16(self.next_used);
    }

    pub(super) fn read(reader: &mut Reader<'_>) -> Result<Self, DeviceStateError> {
        Ok(Self {
            max_size: reader.u16()?,
            size: reader.u16()?,
            ready: reader.presence()?,
            descriptor_address: reader.u64()?,
            available_address: reader.u64()?,
            used_address: reader.u64()?,
            next_available: reader.u16()?,
            next_used: reader.u16()?,
        })
    }
}
