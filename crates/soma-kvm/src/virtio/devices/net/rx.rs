//! Bounded delivery of received frames into guest receive buffers.
//!
//! The loop checks that the driver has posted a buffer before reading from
//! the backend, so a frame is read only when a chain is available, and no
//! chain is popped while the backend is idle. Nothing is held between calls.

use super::frame::{MAX_FRAME_LEN, VIRTIO_NET_HDR_LEN, rx_frame_ok};
use super::{NET_RX_QUEUE, NetDevice};
use crate::virtio::devices::segments::write_writable;
use crate::virtio::devices::service::{DeviceFault, ServiceError, ServiceReport};
use crate::virtio::guest_memory::GuestMemory;
use crate::virtio::queue::violation::QueueViolation;
use crate::virtio::transport::MmioTransport;
use crate::virtio::transport::violation::TransportViolation;

/// Delivers at most `budget` frames while the link is up and buffers exist.
///
/// # Errors
/// Returns why delivery stopped; the transport is already marked on faults.
pub fn deliver_rx<M: GuestMemory + ?Sized>(
    transport: &mut MmioTransport<NetDevice>,
    mem: &M,
    budget: u32,
) -> Result<ServiceReport, ServiceError> {
    if !transport.is_active() {
        return Err(ServiceError::Transport(
            TransportViolation::NotifyBeforeDriverOk,
        ));
    }
    let mut report = ServiceReport::default();
    // One spare byte so an oversized frame is observed rather than cut to the
    // buffer, which is what a TAP read silently does.
    let mut buf = [0u8; VIRTIO_NET_HDR_LEN + MAX_FRAME_LEN + 1];
    for _ in 0..budget {
        let Some((ring, device)) = transport.queue_and_device_mut(NET_RX_QUEUE) else {
            return Err(ServiceError::Transport(
                TransportViolation::NotifyOutOfRange {
                    index: u64::from(NET_RX_QUEUE),
                },
            ));
        };
        if !device.link_up() {
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
        let frame_len = match device.receive_frame(&mut buf[VIRTIO_NET_HDR_LEN..]) {
            Ok(Some(len)) => len,
            Ok(None) => return Ok(report),
            Err(()) => {
                transport.set_needs_reset();
                return Err(ServiceError::Fault(DeviceFault::Backend));
            }
        };
        if !rx_frame_ok(frame_len) {
            device.count_rx_dropped();
            continue;
        }
        let total = VIRTIO_NET_HDR_LEN + frame_len;
        let chain = match ring.pop_descriptor_chain(mem, NetDevice::rx_limits()) {
            Ok(Some(chain)) => chain,
            Ok(None) => return Ok(report),
            Err(QueueViolation::Chain { .. }) => {
                device.count_rx_dropped();
                report.rejected = report.rejected.saturating_add(1);
                continue;
            }
            Err(violation) => {
                transport.set_needs_reset();
                return Err(ServiceError::Queue(violation));
            }
        };
        let fits = chain.readable_len() == 0
            && chain.writable_len() >= u64::try_from(total).unwrap_or(u64::MAX);
        let used = if fits {
            match write_writable(mem, &chain, 0, &buf[..total]) {
                Ok(written) if written == total => {
                    device.count_rx_ok();
                    u32::try_from(total).unwrap_or(u32::MAX)
                }
                _ => {
                    transport.set_needs_reset();
                    return Err(ServiceError::Fault(DeviceFault::Protocol));
                }
            }
        } else {
            device.count_rx_dropped();
            0
        };
        match transport.complete_used(NET_RX_QUEUE, mem, &chain, used) {
            Ok(notify) => report.interrupt |= notify,
            Err(violation) => {
                transport.set_needs_reset();
                return Err(ServiceError::Transport(violation));
            }
        }
        report.completed = report.completed.saturating_add(1);
    }
    report.exhausted = true;
    Ok(report)
}
