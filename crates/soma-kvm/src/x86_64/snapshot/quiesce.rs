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
    if bus.net().device().link_up() {
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
    for slot in Slot::ALL {
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

/// Proves that no ready queue holds a head the device has not taken.
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
    if pending != 0 {
        return Err(SnapshotError::NotQuiescent(
            "a queue holds work the device never took",
        ));
    }
    prove_no_ingress(bus)
}
