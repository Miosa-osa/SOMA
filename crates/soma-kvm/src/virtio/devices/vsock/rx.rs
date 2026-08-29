//! Bounded delivery of host packets and transport events to the guest.
//!
//! A receive chain is popped only when the device has something to send,
//! and the payload of a data packet is sized to the chain's validated
//! writable capacity, so nothing is held between calls and nothing is
//! written past a buffer.

use super::packet::{MAX_PAYLOAD_LEN, VSOCK_HDR_LEN};
use super::{VSOCK_EVENT_QUEUE, VSOCK_RX_QUEUE, VsockDevice};
use crate::virtio::devices::segments::write_writable;
use crate::virtio::devices::service::{DeviceFault, ServiceError, ServiceReport};
use crate::virtio::guest_memory::GuestMemory;
use crate::virtio::queue::chain::DescriptorChain;
use crate::virtio::queue::violation::QueueViolation;
use crate::virtio::transport::MmioTransport;
use crate::virtio::transport::violation::TransportViolation;

/// Delivers at most `budget` packets into posted receive buffers.
///
/// # Errors
/// Returns why delivery stopped; the transport is already marked on faults.
pub fn deliver_rx<M: GuestMemory + ?Sized>(
    transport: &mut MmioTransport<VsockDevice>,
    mem: &M,
    budget: u32,
) -> Result<ServiceReport, ServiceError> {
    deliver(transport, mem, budget, VSOCK_RX_QUEUE, |device, chain| {
        let hdr = u64::try_from(VSOCK_HDR_LEN).unwrap_or(u64::MAX);
        if chain.readable_len() != 0 || chain.writable_len() < hdr {
            device.count_rx_dropped();
            return Some(Vec::new());
        }
        let cap = (chain.writable_len() - hdr).min(u64::from(MAX_PAYLOAD_LEN));
        let cap = usize::try_from(cap).unwrap_or(usize::MAX);
        let (header, payload) = device.next_outbound(cap)?;
        let mut bytes = header.to_bytes().to_vec();
        bytes.extend_from_slice(&payload);
        device.count_rx_packet();
        Some(bytes)
    })
}

/// Delivers queued transport events into posted event buffers.
///
/// # Errors
/// Returns why delivery stopped; the transport is already marked on faults.
pub fn deliver_events<M: GuestMemory + ?Sized>(
    transport: &mut MmioTransport<VsockDevice>,
    mem: &M,
) -> Result<ServiceReport, ServiceError> {
    deliver(transport, mem, 64, VSOCK_EVENT_QUEUE, |device, chain| {
        if chain.readable_len() != 0 || chain.writable_len() < 4 {
            device.count_rx_dropped();
            return Some(Vec::new());
        }
        Some(device.pop_event()?.to_le_bytes().to_vec())
    })
}

/// Shared loop: while the device has work for `queue` and the driver posted
/// a buffer, pop a chain, ask `build` for the bytes, and publish them.
fn deliver<M: GuestMemory + ?Sized>(
    transport: &mut MmioTransport<VsockDevice>,
    mem: &M,
    budget: u32,
    queue: u16,
    mut build: impl FnMut(&mut VsockDevice, &DescriptorChain) -> Option<Vec<u8>>,
) -> Result<ServiceReport, ServiceError> {
    if !transport.is_active() {
        return Err(ServiceError::Transport(
            TransportViolation::NotifyBeforeDriverOk,
        ));
    }
    let mut report = ServiceReport::default();
    for _ in 0..budget {
        let Some((ring, device)) = transport.queue_and_device_mut(queue) else {
            return Err(ServiceError::Transport(
                TransportViolation::NotifyOutOfRange {
                    index: u64::from(queue),
                },
            ));
        };
        let has_work = if queue == VSOCK_EVENT_QUEUE {
            device.pending_events() > 0
        } else {
            device.has_outbound()
        };
        if !has_work {
            return Ok(report);
        }
        match ring.pending(mem) {
            Ok(0) => return Ok(report),
            Ok(_) => {}
            Err(violation) => {
                transport.set_needs_reset();
                return Err(ServiceError::Queue(violation));
            }
        }
        let chain = match ring.pop_descriptor_chain(mem, VsockDevice::chain_limits_for(queue)) {
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
        let Some(bytes) = build(device, &chain) else {
            // Nothing fit this chain; return it empty so the driver reclaims it.
            device.count_rx_dropped();
            complete(transport, mem, queue, &chain, 0, &mut report)?;
            continue;
        };
        let used = match write_writable(mem, &chain, 0, &bytes) {
            Ok(written) if written == bytes.len() => u32::try_from(written).unwrap_or(u32::MAX),
            _ => {
                transport.set_needs_reset();
                return Err(ServiceError::Fault(DeviceFault::Protocol));
            }
        };
        complete(transport, mem, queue, &chain, used, &mut report)?;
    }
    report.exhausted = true;
    Ok(report)
}

fn complete<M: GuestMemory + ?Sized>(
    transport: &mut MmioTransport<VsockDevice>,
    mem: &M,
    queue: u16,
    chain: &DescriptorChain,
    used: u32,
    report: &mut ServiceReport,
) -> Result<(), ServiceError> {
    match transport.complete_used(queue, mem, chain, used) {
        Ok(notify) => {
            report.interrupt |= notify;
            report.completed = report.completed.saturating_add(1);
            Ok(())
        }
        Err(violation) => {
            transport.set_needs_reset();
            Err(ServiceError::Transport(violation))
        }
    }
}
