//! The transport and device restore helpers of the bus, split out to hold their file inside
//! the source budget.
#[expect(
    clippy::wildcard_imports,
    reason = "this module is its parent's body moved wholesale; naming every import would obscure it"
)]
use super::*;

pub(super) fn restore_optional<D: VirtioDevice, M: GuestMemory + ?Sized>(
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

pub(super) fn apply_device<D: VirtioDevice>(
    slot: Slot,
    device: &mut D,
    snapshot: &SlotSnapshot,
) -> Result<(), SlotRestoreError> {
    device
        .restore_state(&snapshot.device)
        .map_err(|error| SlotRestoreError::Device { slot, error })
}

pub(super) fn restore_transport<D: VirtioDevice, M: GuestMemory + ?Sized>(
    slot: Slot,
    device: D,
    snapshot: &SlotSnapshot,
    mem: &M,
) -> Result<MmioTransport<D>, SlotRestoreError> {
    MmioTransport::restore(device, &snapshot.transport, mem)
        .map_err(|error| SlotRestoreError::Transport { slot, error })
}
