//! Bounded servicing of one guest-driven queue for any device model.
//!
//! The loop pops at most `budget` chains, hands each validated chain to the
//! device, publishes the used length, and reports whether the interrupt must
//! be signaled. A hostile chain costs one counter tick; a device fault stops
//! the device with `DEVICE_NEEDS_RESET`.

use std::fmt;

use crate::virtio::device::VirtioDevice;
use crate::virtio::guest_memory::{GuestMemory, GuestMemoryError};
use crate::virtio::queue::chain::{ChainLimits, DescriptorChain};
use crate::virtio::queue::violation::QueueViolation;
use crate::virtio::transport::MmioTransport;
use crate::virtio::transport::violation::TransportViolation;

/// A failure that the device cannot report as an individual request result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceFault {
    /// A validated segment became inaccessible; guest memory is inconsistent.
    Memory(GuestMemoryError),
    /// The host backend failed in a way that leaves the device unusable.
    Backend,
    /// The device state machine was violated in a way the spec does not let
    /// the device answer per request.
    Protocol,
}

impl fmt::Display for DeviceFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "device fault: {self:?}")
    }
}

impl std::error::Error for DeviceFault {}

impl From<GuestMemoryError> for DeviceFault {
    fn from(error: GuestMemoryError) -> Self {
        Self::Memory(error)
    }
}

/// A device model that consumes validated chains from a guest-driven queue.
pub trait ChainHandler {
    /// Host caps applied before a chain from `queue` is walked.
    fn chain_limits(&self, queue: u16) -> ChainLimits;

    /// Processes one validated chain and returns the used length.
    ///
    /// Per-request failures are written into the chain (a status byte) or
    /// dropped with a counter and reported as `Ok(len)`.
    ///
    /// # Errors
    /// Returns a fault only when the device must stop.
    fn handle_chain<M: GuestMemory + ?Sized>(
        &mut self,
        queue: u16,
        chain: &DescriptorChain,
        mem: &M,
    ) -> Result<u32, DeviceFault>;
}

/// What one bounded service pass did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ServiceReport {
    /// Chains handed to the device and published as used.
    pub completed: u32,
    /// Chains rejected by the walker and skipped.
    pub rejected: u32,
    /// Whether the driver wants the device interrupt.
    pub interrupt: bool,
    /// The budget ran out while work may remain; reschedule.
    pub exhausted: bool,
}

/// Why a service pass stopped early; the transport is already marked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceError {
    /// The queue index is invalid or the device is inactive.
    Transport(TransportViolation),
    /// The ring itself is unusable; `DEVICE_NEEDS_RESET` is set.
    Queue(QueueViolation),
    /// The device faulted; `DEVICE_NEEDS_RESET` is set.
    Fault(DeviceFault),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "queue service stopped: {self:?}")
    }
}

impl std::error::Error for ServiceError {}

/// Services `queue` with at most `budget` chains.
///
/// # Errors
/// Returns why servicing stopped; see [`ServiceError`].
pub fn service_queue<D, M>(
    transport: &mut MmioTransport<D>,
    mem: &M,
    queue: u16,
    budget: u32,
) -> Result<ServiceReport, ServiceError>
where
    D: VirtioDevice + ChainHandler,
    M: GuestMemory + ?Sized,
{
    if !transport.is_active() {
        return Err(ServiceError::Transport(
            TransportViolation::NotifyBeforeDriverOk,
        ));
    }
    let mut report = ServiceReport::default();
    let mut remaining = budget;
    loop {
        if remaining == 0 {
            report.exhausted = has_pending(transport, mem, queue)?;
            return Ok(report);
        }
        remaining -= 1;
        let Some((ring, device)) = transport.queue_and_device_mut(queue) else {
            return Err(ServiceError::Transport(
                TransportViolation::NotifyOutOfRange {
                    index: u64::from(queue),
                },
            ));
        };
        let chain = match ring.pop_descriptor_chain(mem, device.chain_limits(queue)) {
            Ok(Some(chain)) => chain,
            Ok(None) => return Ok(report),
            Err(QueueViolation::Chain { .. }) => {
                report.rejected = report.rejected.saturating_add(1);
                continue;
            }
            Err(violation) => {
                transport.set_needs_reset();
                return Err(ServiceError::Queue(violation));
            }
        };
        let len = match device.handle_chain(queue, &chain, mem) {
            Ok(len) => len,
            Err(fault) => {
                transport.set_needs_reset();
                return Err(ServiceError::Fault(fault));
            }
        };
        match transport.complete_used(queue, mem, &chain, len) {
            Ok(notify) => report.interrupt |= notify,
            Err(violation) => {
                transport.set_needs_reset();
                return Err(ServiceError::Transport(violation));
            }
        }
        report.completed = report.completed.saturating_add(1);
    }
}

fn has_pending<D, M>(
    transport: &mut MmioTransport<D>,
    mem: &M,
    queue: u16,
) -> Result<bool, ServiceError>
where
    D: VirtioDevice,
    M: GuestMemory + ?Sized,
{
    let Some((ring, _)) = transport.queue_and_device_mut(queue) else {
        return Ok(false);
    };
    match ring.pending(mem) {
        Ok(pending) => Ok(pending > 0),
        Err(violation) => {
            transport.set_needs_reset();
            Err(ServiceError::Queue(violation))
        }
    }
}
