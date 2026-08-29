//! Modern virtio-mmio (version 2) transport register file.
//!
//! The transport owns the status lifecycle, feature negotiation, queue
//! selection and geometry, interrupt status, and configuration generation.
//! MMIO bus dispatch and KVM exit plumbing sit above this type and call
//! [`MmioTransport::read`] and [`MmioTransport::write`] with page-relative
//! offsets.

mod host;
pub mod registers;
pub mod state;
pub mod status;
pub mod violation;
mod write;

use std::fmt;

use crate::virtio::device::{MAX_CONFIG_LEN, MAX_QUEUES, VIRTIO_F_VERSION_1, VirtioDevice};
use crate::virtio::queue::{Queue, layout::LayoutViolation};
use registers::{AccessWidth, MAGIC_VALUE, MMIO_VERSION, Register};
use status::DeviceStatus;
use violation::{TransportViolation, TransportViolationCounters};

/// `InterruptStatus` bit: a used buffer was published.
pub const INTERRUPT_USED_BUFFER: u32 = 1;
/// `InterruptStatus` bit: the configuration space changed.
pub const INTERRUPT_CONFIG_CHANGE: u32 = 2;
const INTERRUPT_KNOWN: u32 = INTERRUPT_USED_BUFFER | INTERRUPT_CONFIG_CHANGE;

/// A side effect the caller must act on after a write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportEvent {
    /// No caller action is required.
    None,
    /// The driver notified this queue; the device may process it now.
    QueueNotify(u16),
    /// The driver set `DRIVER_OK` and the device activated.
    DriverOk,
    /// The driver wrote status zero; queues and device were reset.
    Reset,
    /// The driver wrote configuration space at this device-relative offset.
    ConfigWritten { offset: u64, len: usize },
}

/// Why a transport could not be constructed from a device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportConfigError {
    /// The allowlist omits `VIRTIO_F_VERSION_1`.
    MissingVersion1,
    /// The device declares no queues or more than [`MAX_QUEUES`].
    QueueCount { count: usize },
    /// A queue maximum is invalid.
    QueueMax {
        index: usize,
        violation: LayoutViolation,
    },
    /// The configuration space exceeds the transport page.
    ConfigTooLarge { len: usize },
}

impl fmt::Display for TransportConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "transport configuration rejected: {self:?}")
    }
}

impl std::error::Error for TransportConfigError {}

/// One virtio-mmio transport page bound to one device model.
pub struct MmioTransport<D: VirtioDevice> {
    device: D,
    queues: Vec<Queue>,
    status: DeviceStatus,
    device_features_sel: u32,
    driver_features_sel: u32,
    driver_features: u64,
    queue_sel: u32,
    interrupt_status: u32,
    config_generation: u32,
    violations: TransportViolationCounters,
}

impl<D: VirtioDevice> MmioTransport<D> {
    /// Binds a device model to a fresh, reset transport.
    ///
    /// # Errors
    /// Rejects a device whose allowlist, queue limits, or config length break v1 rules.
    pub fn new(device: D) -> Result<Self, TransportConfigError> {
        if device.feature_allowlist() & VIRTIO_F_VERSION_1 == 0 {
            return Err(TransportConfigError::MissingVersion1);
        }
        let maxes = device.queue_max_sizes();
        if maxes.is_empty() || maxes.len() > MAX_QUEUES {
            return Err(TransportConfigError::QueueCount { count: maxes.len() });
        }
        if device.config_len() > MAX_CONFIG_LEN {
            return Err(TransportConfigError::ConfigTooLarge {
                len: device.config_len(),
            });
        }
        let mut queues = Vec::with_capacity(maxes.len());
        for (index, &max) in maxes.iter().enumerate() {
            queues.push(
                Queue::new(max)
                    .map_err(|violation| TransportConfigError::QueueMax { index, violation })?,
            );
        }
        Ok(Self {
            device,
            queues,
            status: DeviceStatus::default(),
            device_features_sel: 0,
            driver_features_sel: 0,
            driver_features: 0,
            queue_sel: 0,
            interrupt_status: 0,
            config_generation: 0,
            violations: TransportViolationCounters::default(),
        })
    }

    /// Reads a transport register or configuration byte range.
    ///
    /// # Errors
    /// Returns the typed violation, which is also counted; the bus decides what
    /// value the guest observes for a rejected read.
    pub fn read(&mut self, offset: u64, width: AccessWidth) -> Result<u64, TransportViolation> {
        let register =
            Register::decode(offset).ok_or(TransportViolation::UnknownRegister { offset })?;
        if let Register::Config(config_offset) = register {
            return self.read_config(config_offset, width);
        }
        if width != AccessWidth::U32 {
            return Err(self.record(TransportViolation::WidthMismatch { offset }));
        }
        let value = match register {
            Register::MagicValue => MAGIC_VALUE,
            Register::Version => MMIO_VERSION,
            Register::DeviceId => self.device.device_id(),
            Register::VendorId => self.device.vendor_id(),
            Register::DeviceFeatures => self.device_features_word(),
            Register::QueueNumMax => self
                .selected_queue()
                .map_or(0, |queue| u32::from(queue.max_size())),
            Register::QueueReady => self.selected_queue().is_some_and(Queue::is_ready).into(),
            Register::InterruptStatus => self.interrupt_status,
            Register::Status => u32::from(self.status.bits()),
            // No shared-memory regions exist: the spec requires all-ones.
            Register::ShmLenLow
            | Register::ShmLenHigh
            | Register::ShmBaseLow
            | Register::ShmBaseHigh => u32::MAX,
            Register::QueueReset => 0,
            Register::ConfigGeneration => self.config_generation,
            _ => return Err(self.record(TransportViolation::ReadOfWriteOnly { offset })),
        };
        Ok(u64::from(value))
    }

    fn device_features_word(&self) -> u32 {
        let allowlist = self.device.feature_allowlist();
        match self.device_features_sel {
            0 => (allowlist & 0xffff_ffff) as u32,
            1 => (allowlist >> 32) as u32,
            _ => 0,
        }
    }

    fn read_config(&mut self, offset: u64, width: AccessWidth) -> Result<u64, TransportViolation> {
        let start = self.config_range(offset, width.bytes())?;
        let mut raw = [0u8; 8];
        let buf = &mut raw[..width.bytes()];
        self.device
            .read_config(start, buf)
            .map_err(|error| self.record(TransportViolation::ConfigAccess(error)))?;
        Ok(u64::from_le_bytes(raw))
    }

    fn config_range(&mut self, offset: u64, len: usize) -> Result<usize, TransportViolation> {
        let start = usize::try_from(offset).ok();
        let end = start.and_then(|start| start.checked_add(len));
        match (start, end) {
            (Some(start), Some(end)) if end <= self.device.config_len() => Ok(start),
            _ => Err(self.record(TransportViolation::ConfigOutOfBounds { offset })),
        }
    }

    fn selected_queue(&self) -> Option<&Queue> {
        usize::try_from(self.queue_sel)
            .ok()
            .and_then(|index| self.queues.get(index))
    }

    fn record(&mut self, violation: TransportViolation) -> TransportViolation {
        self.violations.record(&violation);
        violation
    }

    /// Whether the lifecycle allows queue work: `DRIVER_OK` and no failure bit.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.status.driver_ok() && !self.status.is_failed()
    }

    /// Current status register.
    #[must_use]
    pub const fn status(&self) -> DeviceStatus {
        self.status
    }

    /// Feature word the driver accepted; meaningful once `FEATURES_OK` is set.
    #[must_use]
    pub const fn driver_features(&self) -> u64 {
        self.driver_features
    }

    /// Pending interrupt bits.
    #[must_use]
    pub const fn interrupt_status(&self) -> u32 {
        self.interrupt_status
    }

    /// Bounded violation counters.
    #[must_use]
    pub const fn violations(&self) -> &TransportViolationCounters {
        &self.violations
    }

    /// The bound device model.
    #[must_use]
    pub const fn device(&self) -> &D {
        &self.device
    }

    /// Mutable access to the bound device model.
    pub const fn device_mut(&mut self) -> &mut D {
        &mut self.device
    }

    /// Queue by index.
    #[must_use]
    pub fn queue(&self, index: u16) -> Option<&Queue> {
        self.queues.get(usize::from(index))
    }

    /// Queue and device together, for device work that reads and completes chains.
    pub fn queue_and_device_mut(&mut self, index: u16) -> Option<(&mut Queue, &mut D)> {
        let queue = self.queues.get_mut(usize::from(index))?;
        Some((queue, &mut self.device))
    }

    fn reset_all(&mut self) {
        self.device.reset();
        for queue in &mut self.queues {
            queue.reset();
        }
        self.status = DeviceStatus::default();
        self.device_features_sel = 0;
        self.driver_features_sel = 0;
        self.driver_features = 0;
        self.queue_sel = 0;
        self.interrupt_status = 0;
    }
}

#[cfg(test)]
mod driver_model_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod restore_tests;
#[cfg(test)]
mod tests;
