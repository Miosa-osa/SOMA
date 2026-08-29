//! Per-slot queue servicing, inbound delivery, and snapshot records.

use std::fmt;

use super::{BusDevices, MmioBus, Slot, with_slot};
use crate::virtio::device::{DeviceStateError, VirtioDevice};
use crate::virtio::devices::net::{NET_RX_QUEUE, rx::deliver_rx};
use crate::virtio::devices::service::{ServiceError, ServiceReport, service_queue};
use crate::virtio::devices::vsock::rx::{deliver_events, deliver_rx as deliver_vsock_rx};
use crate::virtio::devices::vsock::{VSOCK_EVENT_QUEUE, VSOCK_RX_QUEUE};
use crate::virtio::guest_memory::GuestMemory;
use crate::virtio::queue::violation::QueueViolation;
use crate::virtio::transport::MmioTransport;
use crate::virtio::transport::state::{RestoreError, TransportState};
use crate::virtio::transport::violation::TransportViolation;

/// Captured state of one slot: transport record plus device record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlotSnapshot {
    pub slot: Slot,
    pub transport: TransportState,
    pub device: Vec<u8>,
}

/// Why a slot could not be restored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotRestoreError {
    /// The record names a different slot than its position.
    SlotMismatch { expected: Slot, actual: Slot },
    /// The device record was rejected.
    Device { slot: Slot, error: DeviceStateError },
    /// The transport record was rejected.
    Transport { slot: Slot, error: RestoreError },
}

impl fmt::Display for SlotRestoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "slot restore rejected: {self:?}")
    }
}

impl std::error::Error for SlotRestoreError {}

impl MmioBus {
    /// Services a driver notification for `(slot, queue)` with a work budget.
    ///
    /// Guest-driven queues run the device parser; receive and event queues
    /// deliver whatever the host side already has, and a vsock transmit pass
    /// also flushes the replies it produced.
    ///
    /// # Errors
    /// Returns why servicing stopped; the transport is already marked.
    pub fn service<M: GuestMemory + ?Sized>(
        &mut self,
        slot: Slot,
        queue: u16,
        mem: &M,
        budget: u32,
    ) -> Result<ServiceReport, ServiceError> {
        if queue >= slot.queue_count() {
            return Err(ServiceError::Transport(
                TransportViolation::NotifyOutOfRange {
                    index: u64::from(queue),
                },
            ));
        }
        match (slot, queue) {
            (Slot::Root, _) => service_queue(&mut self.root, mem, queue, budget),
            (Slot::Overlay, _) => service_queue(&mut self.overlay, mem, queue, budget),
            (Slot::Net, NET_RX_QUEUE) => deliver_rx(&mut self.net, mem, budget),
            (Slot::Net, _) => service_queue(&mut self.net, mem, queue, budget),
            (Slot::Vsock, VSOCK_RX_QUEUE) => deliver_vsock_rx(&mut self.vsock, mem, budget),
            (Slot::Vsock, VSOCK_EVENT_QUEUE) => deliver_events(&mut self.vsock, mem),
            (Slot::Vsock, _) => {
                let mut report = service_queue(&mut self.vsock, mem, queue, budget)?;
                let replies = deliver_vsock_rx(&mut self.vsock, mem, budget)?;
                report.interrupt |= replies.interrupt;
                report.completed = report.completed.saturating_add(replies.completed);
                Ok(report)
            }
            (Slot::Rng, _) => service_queue(&mut self.rng, mem, queue, budget),
        }
    }

    /// Delivers host-originated work (frames, vsock packets, events) for a slot.
    ///
    /// # Errors
    /// Returns why delivery stopped; the transport is already marked.
    pub fn deliver_inbound<M: GuestMemory + ?Sized>(
        &mut self,
        slot: Slot,
        mem: &M,
        budget: u32,
    ) -> Result<ServiceReport, ServiceError> {
        match slot {
            Slot::Net => deliver_rx(&mut self.net, mem, budget),
            Slot::Vsock => {
                let mut report = deliver_vsock_rx(&mut self.vsock, mem, budget)?;
                let events = deliver_events(&mut self.vsock, mem)?;
                report.interrupt |= events.interrupt;
                report.completed = report.completed.saturating_add(events.completed);
                Ok(report)
            }
            Slot::Root | Slot::Overlay | Slot::Rng => Ok(ServiceReport::default()),
        }
    }

    /// Counts driver-posted heads no device has taken yet, across every ready queue.
    ///
    /// Snapshot capture requires this to be zero: a queue with unserviced work would be
    /// restored into an Instance that never asked for it.
    ///
    /// # Errors
    /// Returns the first queue violation found while reading the available ring.
    pub fn pending_work<M: GuestMemory + ?Sized>(
        &mut self,
        mem: &M,
    ) -> Result<u32, (Slot, u16, QueueViolation)> {
        let mut pending = 0_u32;
        for slot in Slot::ALL {
            for index in 0..slot.queue_count() {
                let queue = with_slot!(self, slot, |t| t
                    .queue_and_device_mut(index)
                    .map(|(queue, _)| queue.is_ready().then(|| queue.pending(mem))));
                match queue {
                    Some(Some(Ok(count))) => pending = pending.saturating_add(u32::from(count)),
                    Some(Some(Err(violation))) => return Err((slot, index, violation)),
                    Some(None) | None => {}
                }
            }
        }
        Ok(pending)
    }

    /// Captures one slot; the caller proves quiescence first.
    #[must_use]
    pub fn snapshot(&mut self, slot: Slot) -> SlotSnapshot {
        let (transport, device) =
            with_slot!(self, slot, |t| (t.state(), t.device().snapshot_state()));
        SlotSnapshot {
            slot,
            transport,
            device,
        }
    }

    /// Captures all five slots in table order.
    #[must_use]
    pub fn snapshot_all(&mut self) -> Vec<SlotSnapshot> {
        Slot::ALL.iter().map(|slot| self.snapshot(*slot)).collect()
    }

    /// Rebuilds a bus from fresh devices and captured records, validating
    /// device identity before transport state and failing closed on any
    /// mismatch.
    ///
    /// # Errors
    /// Returns the first rejected slot in table order.
    pub fn restore<M: GuestMemory + ?Sized>(
        devices: BusDevices,
        snapshots: &[SlotSnapshot; super::SLOT_COUNT],
        mem: &M,
    ) -> Result<Self, SlotRestoreError> {
        let BusDevices {
            mut root,
            mut overlay,
            mut net,
            mut vsock,
            mut rng,
        } = devices;
        for (expected, snapshot) in Slot::ALL.iter().zip(snapshots) {
            if snapshot.slot != *expected {
                return Err(SlotRestoreError::SlotMismatch {
                    expected: *expected,
                    actual: snapshot.slot,
                });
            }
        }
        apply_device(Slot::Root, &mut root, &snapshots[0])?;
        apply_device(Slot::Overlay, &mut overlay, &snapshots[1])?;
        apply_device(Slot::Net, &mut net, &snapshots[2])?;
        apply_device(Slot::Vsock, &mut vsock, &snapshots[3])?;
        apply_device(Slot::Rng, &mut rng, &snapshots[4])?;
        Ok(Self {
            root: restore_transport(Slot::Root, root, &snapshots[0], mem)?,
            overlay: restore_transport(Slot::Overlay, overlay, &snapshots[1], mem)?,
            net: restore_transport(Slot::Net, net, &snapshots[2], mem)?,
            vsock: restore_transport(Slot::Vsock, vsock, &snapshots[3], mem)?,
            rng: restore_transport(Slot::Rng, rng, &snapshots[4], mem)?,
        })
    }
}

fn apply_device<D: VirtioDevice>(
    slot: Slot,
    device: &mut D,
    snapshot: &SlotSnapshot,
) -> Result<(), SlotRestoreError> {
    device
        .restore_state(&snapshot.device)
        .map_err(|error| SlotRestoreError::Device { slot, error })
}

fn restore_transport<D: VirtioDevice, M: GuestMemory + ?Sized>(
    slot: Slot,
    device: D,
    snapshot: &SlotSnapshot,
    mem: &M,
) -> Result<MmioTransport<D>, SlotRestoreError> {
    MmioTransport::restore(device, &snapshot.transport, mem)
        .map_err(|error| SlotRestoreError::Transport { slot, error })
}
