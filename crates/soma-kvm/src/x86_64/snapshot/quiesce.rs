//! The device-side quiesce proofs the device-surface contract requires at the capture point.
//!
//! Each check is a statement about the live devices, not a hope about them: the network link
//! is down and no frame is buffered, vsock holds no connection, credit, packet, or event, the
//! entropy device holds no request, and no queue has a driver-posted head that no device has
//! taken. A capture that cannot prove all of them aborts instead of guessing later.

use super::error::SnapshotError;
use crate::virtio::{GuestMemory, MmioBus, Slot};

/// Bounded work per drain pass; the same budget the device thread uses.
const BUDGET: u32 = 64;
/// Passes before a queue that keeps producing work is treated as not quiescent.
const PASSES: u32 = 8;

/// Proves that nothing can enter the machine from the host side.
///
/// # Errors
///
/// Returns [`SnapshotError::NotQuiescent`] naming the first device that is not idle.
pub(super) fn prove_no_ingress(bus: &MmioBus) -> Result<(), SnapshotError> {
    // A machine with no network device has no link that could be up, which is the strongest
    // form of the same statement rather than an unchecked one.
    if bus.net().is_some_and(|net| net.device().link_up()) {
        return Err(SnapshotError::NotQuiescent(
            "the network link is up at the capture point",
        ));
    }
    if !bus.vsock().device().is_quiescent() {
        return Err(SnapshotError::NotQuiescent(
            "the vsock device holds a connection, packet, or event",
        ));
    }
    Ok(())
}

/// Runs the last bounded servicing pass the stopped device thread would have run.
///
/// # Errors
///
/// Returns [`SnapshotError::NotQuiescent`] when a device faults or a queue keeps producing
/// work after the pass limit.
pub(super) fn drain<M: GuestMemory + ?Sized>(
    bus: &mut MmioBus,
    memory: &M,
) -> Result<(), SnapshotError> {
    for slot in bus.device_set().present().collect::<Vec<_>>() {
        for queue in 0..slot.queue_count() {
            let mut passes = 0;
            loop {
                let report = bus.service(slot, queue, memory, BUDGET).map_err(|_| {
                    SnapshotError::NotQuiescent("a device faulted during the final drain")
                })?;
                passes += 1;
                if !report.exhausted {
                    break;
                }
                if passes >= PASSES {
                    return Err(SnapshotError::NotQuiescent(
                        "a queue still had work after the final drain",
                    ));
                }
            }
        }
    }
    Ok(())
}

/// The queues the guest drives, which must hold no unserviced head at the capture point.
///
/// The receive and event queues are excluded on purpose: the driver posts empty buffers into
/// them in advance and the device fills them only when the host has something to deliver, so a
/// nonzero count there is ordinary posted capacity that restore carries forward unchanged.
const GUEST_DRIVEN: [(Slot, u16); 4] = [
    (Slot::Root, 0),
    (Slot::Overlay, 0),
    (Slot::Net, crate::virtio::NET_TX_QUEUE),
    (Slot::Rng, 0),
];

/// Proves that no guest-driven queue holds a head the device has not taken.
///
/// # Errors
///
/// Returns [`SnapshotError::NotQuiescent`] for pending work, a queue violation, or a device
/// that reacquired ingress state during the drain.
pub(super) fn prove_queues_quiescent<M: GuestMemory + ?Sized>(
    bus: &mut MmioBus,
    memory: &M,
) -> Result<(), SnapshotError> {
    let pending = bus.pending_work(memory).map_err(|_| {
        SnapshotError::NotQuiescent("a queue could not be read while proving quiescence")
    })?;
    for (slot, queue) in GUEST_DRIVEN {
        if pending[usize::from(slot.index())][usize::from(queue)] != 0 {
            return Err(SnapshotError::NotQuiescent(
                "a guest-driven queue holds work the device never took",
            ));
        }
    }
    // The vsock transmit queue is guest-driven too, but the control device is proven to hold
    // no connection at all, which is the stronger statement the device contract asks for.
    if pending[usize::from(Slot::Vsock.index())][usize::from(crate::virtio::VSOCK_TX_QUEUE)] != 0 {
        return Err(SnapshotError::NotQuiescent(
            "the vsock transmit queue holds a packet the device never took",
        ));
    }
    prove_no_ingress(bus)
}

/// The posted receive and event capacity a capture carries forward, for the evidence.
pub(super) fn posted<M: GuestMemory + ?Sized>(bus: &mut MmioBus, memory: &M) -> [u32; 3] {
    bus.pending_work(memory).map_or([0; 3], |pending| {
        [
            pending[usize::from(Slot::Net.index())][usize::from(crate::virtio::NET_RX_QUEUE)],
            pending[usize::from(Slot::Vsock.index())][usize::from(crate::virtio::VSOCK_RX_QUEUE)],
            pending[usize::from(Slot::Vsock.index())]
                [usize::from(crate::virtio::VSOCK_EVENT_QUEUE)],
        ]
    })
}
