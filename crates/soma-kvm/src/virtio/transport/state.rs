//! Serializable transport state for snapshot capture and validated restore.
//!
//! The encoding is a fixed 30-byte header followed by one 32-byte
//! [`QueueState`] record per queue, all little-endian.

use std::fmt;

use crate::virtio::device::{MAX_QUEUES, VirtioDevice};
use crate::virtio::guest_memory::GuestMemory;
use crate::virtio::queue::Queue;
use crate::virtio::queue::state::{QUEUE_STATE_LEN, QueueState, QueueStateError, le_u64};
use crate::virtio::transport::status::{DeviceStatus, StatusViolation};
use crate::virtio::transport::violation::TransportViolation;
use crate::virtio::transport::{INTERRUPT_KNOWN, MmioTransport, TransportConfigError};

/// Encoded header length before the queue records.
pub const TRANSPORT_STATE_HEADER_LEN: usize = 30;

/// Snapshot-visible transport state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TransportState {
    pub status: u8,
    pub device_features_sel: u32,
    pub driver_features_sel: u32,
    pub driver_features: u64,
    pub queue_sel: u32,
    pub interrupt_status: u32,
    pub config_generation: u32,
    pub queues: Vec<QueueState>,
}

/// Why an encoded transport state was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportStateError {
    /// The record is shorter than the header or not header plus whole queue records.
    Length { actual: usize },
    /// The queue count exceeds [`MAX_QUEUES`].
    QueueCount { count: u8 },
    /// A queue record was rejected.
    Queue {
        index: usize,
        error: QueueStateError,
    },
}

impl fmt::Display for TransportStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "transport state rejected: {self:?}")
    }
}

impl std::error::Error for TransportStateError {}

impl TransportState {
    /// Encodes the state.
    ///
    /// # Errors
    /// Rejects more than [`MAX_QUEUES`] queue records.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TransportStateError> {
        let count = u8::try_from(self.queues.len())
            .ok()
            .filter(|count| usize::from(*count) <= MAX_QUEUES)
            .ok_or(TransportStateError::QueueCount { count: u8::MAX })?;
        let mut raw = Vec::with_capacity(TRANSPORT_STATE_HEADER_LEN + 32 * self.queues.len());
        raw.push(self.status);
        raw.extend_from_slice(&self.device_features_sel.to_le_bytes());
        raw.extend_from_slice(&self.driver_features_sel.to_le_bytes());
        raw.extend_from_slice(&self.driver_features.to_le_bytes());
        raw.extend_from_slice(&self.queue_sel.to_le_bytes());
        raw.extend_from_slice(&self.interrupt_status.to_le_bytes());
        raw.extend_from_slice(&self.config_generation.to_le_bytes());
        raw.push(count);
        for queue in &self.queues {
            raw.extend_from_slice(&queue.to_bytes());
        }
        Ok(raw)
    }

    /// Decodes with exact-length and bounded-count checks.
    ///
    /// # Errors
    /// Rejects wrong lengths, oversized queue counts, and bad queue records.
    pub fn from_bytes(raw: &[u8]) -> Result<Self, TransportStateError> {
        let length = TransportStateError::Length { actual: raw.len() };
        if raw.len() < TRANSPORT_STATE_HEADER_LEN {
            return Err(length);
        }
        let count = raw[29];
        if usize::from(count) > MAX_QUEUES {
            return Err(TransportStateError::QueueCount { count });
        }
        if raw.len() != TRANSPORT_STATE_HEADER_LEN + usize::from(count) * QUEUE_STATE_LEN {
            return Err(length);
        }
        let u32_at = |offset: usize| u32::try_from(le_u64(&raw[offset..offset + 4])).unwrap_or(0);
        let mut queues = Vec::with_capacity(usize::from(count));
        for index in 0..usize::from(count) {
            let start = TRANSPORT_STATE_HEADER_LEN + index * QUEUE_STATE_LEN;
            let queue = QueueState::from_bytes(&raw[start..start + QUEUE_STATE_LEN])
                .map_err(|error| TransportStateError::Queue { index, error })?;
            queues.push(queue);
        }
        Ok(Self {
            status: raw[0],
            device_features_sel: u32_at(1),
            driver_features_sel: u32_at(5),
            driver_features: le_u64(&raw[9..17]),
            queue_sel: u32_at(17),
            interrupt_status: u32_at(21),
            config_generation: u32_at(25),
            queues,
        })
    }
}

/// Why a captured transport state could not be restored onto a device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreError {
    /// The device itself is not a valid v1 transport device.
    Config(TransportConfigError),
    /// The captured status bits are invalid or out of order.
    Status(StatusViolation),
    /// The queue count does not match the device.
    QueueCount { expected: usize, actual: usize },
    /// Interrupt status has bits the transport never raises.
    InterruptStatus { value: u32 },
    /// Negotiated features or activation were rejected.
    Transport(TransportViolation),
    /// A queue could not be restored.
    Queue {
        index: usize,
        violation: TransportViolation,
    },
}

impl fmt::Display for RestoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "transport restore rejected: {self:?}")
    }
}

impl std::error::Error for RestoreError {}

impl<D: VirtioDevice> MmioTransport<D> {
    /// Snapshot-visible state; device-specific bytes come from the device.
    #[must_use]
    pub fn state(&self) -> TransportState {
        TransportState {
            status: self.status.bits(),
            device_features_sel: self.device_features_sel,
            driver_features_sel: self.driver_features_sel,
            driver_features: self.driver_features,
            queue_sel: self.queue_sel,
            interrupt_status: self.interrupt_status,
            config_generation: self.config_generation,
            queues: self.queues.iter().map(Queue::state).collect(),
        }
    }

    /// Rebuilds a transport from captured state, revalidating every invariant
    /// and re-activating the device when `DRIVER_OK` was captured.
    ///
    /// # Errors
    /// Fails closed on any status, feature, queue, or activation mismatch.
    pub fn restore<M: GuestMemory + ?Sized>(
        device: D,
        state: &TransportState,
        mem: &M,
    ) -> Result<Self, RestoreError> {
        let mut transport = Self::new(device).map_err(RestoreError::Config)?;
        let status = DeviceStatus::from_bits(state.status).map_err(RestoreError::Status)?;
        if state.queues.len() != transport.queues.len() {
            return Err(RestoreError::QueueCount {
                expected: transport.queues.len(),
                actual: state.queues.len(),
            });
        }
        if state.interrupt_status & !INTERRUPT_KNOWN != 0 {
            return Err(RestoreError::InterruptStatus {
                value: state.interrupt_status,
            });
        }
        transport.driver_features = state.driver_features;
        if status.features_ok() {
            transport
                .check_features()
                .map_err(RestoreError::Transport)?;
        }
        for (index, queue_state) in state.queues.iter().enumerate() {
            let max = transport.queues[index].max_size();
            if queue_state.ready && !status.features_ok() {
                return Err(RestoreError::Queue {
                    index,
                    violation: TransportViolation::ConfigurationLocked { offset: 0 },
                });
            }
            transport.queues[index] =
                Queue::restore(mem, max, *queue_state).map_err(|violation| {
                    RestoreError::Queue {
                        index,
                        violation: TransportViolation::Queue(violation),
                    }
                })?;
        }
        if status.driver_ok() {
            transport
                .device
                .activate(state.driver_features)
                .map_err(|error| RestoreError::Transport(TransportViolation::Activate(error)))?;
        }
        transport.status = status;
        transport.device_features_sel = state.device_features_sel;
        transport.driver_features_sel = state.driver_features_sel;
        transport.queue_sel = state.queue_sel;
        transport.interrupt_status = state.interrupt_status;
        transport.config_generation = state.config_generation;
        Ok(transport)
    }
}
