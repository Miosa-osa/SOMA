//! Per-slot queue servicing, inbound delivery, and snapshot records.

use std::fmt;

use super::{BusDevices, MmioBus, SLOT_COUNT, Slot, with_slot};
use crate::virtio::device::{DeviceStateError, MAX_QUEUES, VirtioDevice};
use crate::virtio::devices::net::{NET_RX_QUEUE, rx::deliver_rx};
use crate::virtio::devices::service::{ServiceError, ServiceReport, service_queue};
use crate::virtio::devices::vsock::rx::{deliver_events, deliver_rx as deliver_vsock_rx};
use crate::virtio::devices::vsock::{VSOCK_EVENT_QUEUE, VSOCK_RX_QUEUE};
use crate::virtio::guest_memory::GuestMemory;
use crate::virtio::queue::violation::QueueViolation;
use crate::virtio::transport::MmioTransport;
use crate::virtio::transport::state::{RestoreError, TransportState};
use crate::virtio::transport::violation::TransportViolation;

/// Driver-posted heads per slot and queue, indexed by [`Slot::index`] and queue index.
pub type PendingWork = [[u32; MAX_QUEUES]; SLOT_COUNT];

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
    /// The snapshot was captured from a machine with a different set of devices.
    SlotSetMismatch { expected: usize, actual: usize },
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
        // An absent slot has no queue and no registered ioeventfd, so nothing can legitimately
        // notify it; there is no work to do and none is reported as done.
        match (slot, queue) {
            (Slot::Root, _) => service_queue(&mut self.root, mem, queue, budget),
            (Slot::Overlay, _) => match self.overlay.as_mut() {
                Some(overlay) => service_queue(overlay, mem, queue, budget),
                None => Ok(ServiceReport::default()),
            },
            (Slot::Net, NET_RX_QUEUE) => match self.net.as_mut() {
                Some(net) => deliver_rx(net, mem, budget),
                None => Ok(ServiceReport::default()),
            },
            (Slot::Net, _) => match self.net.as_mut() {
                Some(net) => service_queue(net, mem, queue, budget),
                None => Ok(ServiceReport::default()),
            },
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
            Slot::Net => match self.net.as_mut() {
                Some(net) => deliver_rx(net, mem, budget),
                None => Ok(ServiceReport::default()),
            },
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

    /// Counts driver-posted heads no device has taken yet, per slot and queue.
    ///
    /// A guest-driven queue must read zero at the capture point: work the device never took
    /// would be restored into an Instance that never asked for it. A device-filled receive or
    /// event queue reads the number of buffers the driver posted in advance, which is ordinary
    /// state and is restored with the queue.
    ///
    /// # Errors
    /// Returns the first queue violation found while reading the available ring.
    pub fn pending_work<M: GuestMemory + ?Sized>(
        &mut self,
        mem: &M,
    ) -> Result<PendingWork, (Slot, u16, QueueViolation)> {
        let mut pending = PendingWork::default();
        for slot in Slot::ALL {
            for index in 0..slot.queue_count() {
                // The outer level is slot presence and the next is whether the transport has
                // that queue at all; both mean the same thing here, so they flatten together.
                let queue = with_slot!(self, slot, |t| t
                    .queue_and_device_mut(index)
                    .map(|(queue, _)| queue.is_ready().then(|| queue.pending(mem))))
                .flatten();
                match queue {
                    Some(Some(Ok(count))) => {
                        pending[usize::from(slot.index())][usize::from(index)] = u32::from(count);
                    }
                    Some(Some(Err(violation))) => return Err((slot, index, violation)),
                    Some(None) | None => {}
                }
            }
        }
        Ok(pending)
    }

    /// Captures one slot; the caller proves quiescence first.
    ///
    /// An absent slot has no transport and no device state, so there is nothing to capture and
    /// nothing a restore would have to reproduce.
    #[must_use]
    pub fn snapshot(&mut self, slot: Slot) -> Option<SlotSnapshot> {
        let (transport, device) =
            with_slot!(self, slot, |t| (t.state(), t.device().snapshot_state()))?;
        Some(SlotSnapshot {
            slot,
            transport,
            device,
        })
    }

    /// Captures every present slot in table order.
    #[must_use]
    pub fn snapshot_all(&mut self) -> Vec<SlotSnapshot> {
        self.device_set()
            .present()
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|slot| self.snapshot(slot))
            .collect()
    }

    /// Rebuilds a bus from fresh devices and captured records, validating
    /// device identity before transport state and failing closed on any
    /// mismatch.
    ///
    /// # Errors
    /// Returns the first rejected slot in table order.
    /// The records must name exactly the present slots, in table order: a snapshot taken from a
    /// machine with a different device set is a different machine, and restoring one into the
    /// other would give the guest a device its captured driver never negotiated.
    pub fn restore<M: GuestMemory + ?Sized>(
        devices: BusDevices,
        snapshots: &[SlotSnapshot],
        mem: &M,
    ) -> Result<Self, SlotRestoreError> {
        let present = devices.device_set().present().collect::<Vec<_>>();
        if snapshots.len() != present.len() {
            return Err(SlotRestoreError::SlotSetMismatch {
                expected: present.len(),
                actual: snapshots.len(),
            });
        }
        for (expected, record) in present.iter().zip(snapshots) {
            if record.slot != *expected {
                return Err(SlotRestoreError::SlotMismatch {
                    expected: *expected,
                    actual: record.slot,
                });
            }
        }
        let record = |slot: Slot| snapshots.iter().find(|record| record.slot == slot);
        let BusDevices {
            mut root,
            mut overlay,
            mut net,
            mut vsock,
            mut rng,
        } = devices;
        let (Some(root_record), Some(vsock_record), Some(rng_record)) =
            (record(Slot::Root), record(Slot::Vsock), record(Slot::Rng))
        else {
            return Err(SlotRestoreError::SlotSetMismatch {
                expected: present.len(),
                actual: snapshots.len(),
            });
        };
        let overlay_record = record(Slot::Overlay);
        let net_record = record(Slot::Net);
        apply_device(Slot::Root, &mut root, root_record)?;
        if let (Some(device), Some(record)) = (overlay.as_mut(), overlay_record) {
            apply_device(Slot::Overlay, device, record)?;
        }
        if let (Some(device), Some(record)) = (net.as_mut(), net_record) {
            apply_device(Slot::Net, device, record)?;
        }
        apply_device(Slot::Vsock, &mut vsock, vsock_record)?;
        apply_device(Slot::Rng, &mut rng, rng_record)?;
        Ok(Self {
            root: restore_transport(Slot::Root, root, root_record, mem)?,
            overlay: restore_optional(Slot::Overlay, overlay, overlay_record, mem)?,
            net: restore_optional(Slot::Net, net, net_record, mem)?,
            vsock: restore_transport(Slot::Vsock, vsock, vsock_record, mem)?,
            rng: restore_transport(Slot::Rng, rng, rng_record, mem)?,
        })
    }
}

fn restore_optional<D: VirtioDevice, M: GuestMemory + ?Sized>(
    slot: Slot,
    device: Option<D>,
    snapshot: Option<&SlotSnapshot>,
    mem: &M,
) -> Result<Option<MmioTransport<D>>, SlotRestoreError> {
    match (device, snapshot) {
        (Some(device), Some(snapshot)) => restore_transport(slot, device, snapshot, mem).map(Some),
        _ => Ok(None),
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
