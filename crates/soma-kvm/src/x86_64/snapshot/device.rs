//! The bridge between the live virtio slot records and the canonical device sections.
//!
//! The manifest carries the canonical form so compatibility, bounds, and integrity are
//! checked by the codec rather than by the device implementation. Two live values have no
//! canonical field: the two feature-selector registers, which are transient negotiation state
//! with no meaning once the driver has set `DRIVER_OK`, and the sticky activation flag, which
//! restore re-derives from readiness and the queue cursors. Capture proves that the canonical
//! form reproduces everything else exactly.

use crate::snapshot::{
    Digest,
    device_state::{
        BlockState as CanonicalBlock, DeviceKind, DeviceSpecific, DeviceState, QueueState,
        TransportState as CanonicalTransport,
    },
};
use crate::virtio::{
    BusDevices, MmioBus, QueueState as LiveQueue, Slot, SlotSnapshot,
    TransportState as LiveTransport, VirtioDevice,
};

use super::{error::SnapshotError, profile};

/// The manifest section a device slot occupies.
///
/// The mapping is by slot rather than by position in the captured list, so a manifest that
/// leaves a slot out still puts every other device in the section a restore looks for it in.
pub(super) fn role(slot: Slot) -> Option<crate::snapshot::section::SectionRole> {
    crate::snapshot::section::SectionRole::ALL
        .into_iter()
        .find(|role| role.device_slot() == Some(slot.index()))
}

/// Reads the device-specific fields of one slot from the live bus.
///
/// A slot the machine does not have has no fields to read; the caller captures no section for
/// it, so `None` here and a missing section in the manifest are the same statement.
pub(super) fn specific(bus: &MmioBus, slot: Slot, image: Digest) -> Option<DeviceSpecific> {
    Some(match slot {
        Slot::Root => DeviceSpecific::Block(CanonicalBlock {
            capacity_sectors: bus.root().device().capacity_sectors(),
            block_size: bus.root().device().blk_size(),
            image_digest: image,
        }),
        Slot::Overlay => DeviceSpecific::Block(CanonicalBlock {
            capacity_sectors: bus.overlay()?.device().capacity_sectors(),
            block_size: bus.overlay()?.device().blk_size(),
            image_digest: image,
        }),
        Slot::Net => DeviceSpecific::Net {
            mac: bus.net()?.device().mac(),
            link_up: bus.net()?.device().link_up(),
        },
        Slot::Vsock => DeviceSpecific::Vsock {
            cid_placeholder: bus.vsock().device().guest_cid(),
        },
        Slot::Rng => DeviceSpecific::Rng,
    })
}

/// Converts one live slot record into its canonical section value.
///
/// # Errors
///
/// Returns [`SnapshotError::FeatureNegotiation`] when the driver negotiated anything other
/// than this implementation's allowlist, or the canonical validation failure.
pub(super) fn canonical(
    slot: Slot,
    live: &SlotSnapshot,
    specific: DeviceSpecific,
) -> Result<DeviceState, SnapshotError> {
    let expectation = profile::expectation(slot);
    if live.transport.driver_features != expectation.negotiated_features {
        return Err(SnapshotError::FeatureNegotiation {
            slot,
            negotiated: live.transport.driver_features,
        });
    }
    let queues = live
        .transport
        .queues
        .iter()
        .zip(expectation.queue_limits)
        .map(|(queue, max_size)| QueueState {
            max_size,
            size: queue.size,
            ready: queue.ready,
            descriptor_address: queue.desc,
            available_address: queue.avail,
            used_address: queue.used,
            next_available: queue.next_avail,
            next_used: queue.next_used,
        })
        .collect();
    Ok(DeviceState::new(
        expectation.kind,
        CanonicalTransport {
            device_status: live.transport.status,
            interrupt_status: u8::try_from(live.transport.interrupt_status).unwrap_or(u8::MAX),
            config_generation: live.transport.config_generation,
            queue_select: u16::try_from(live.transport.queue_sel).unwrap_or(u16::MAX),
        },
        live.transport.driver_features,
        queues,
        specific,
    )?)
}

/// Converts one canonical section value back into the live transport record.
pub(super) fn transport(state: &DeviceState) -> LiveTransport {
    LiveTransport {
        status: state.transport().device_status,
        device_features_sel: 0,
        driver_features_sel: 0,
        driver_features: state.negotiated_features(),
        queue_sel: u32::from(state.transport().queue_select),
        interrupt_status: u32::from(state.transport().interrupt_status),
        config_generation: state.transport().config_generation,
        queues: state
            .queues()
            .iter()
            .map(|queue| LiveQueue {
                size: queue.size,
                ready: queue.ready,
                activated: queue.ready || queue.next_available != 0 || queue.next_used != 0,
                desc: queue.descriptor_address,
                avail: queue.available_address,
                used: queue.used_address,
                next_avail: queue.next_available,
                next_used: queue.next_used,
            })
            .collect(),
    }
}

/// Proves that the canonical form reproduces the live record apart from the two documented
/// transient values.
pub(super) fn reproduces(slot: Slot, live: &SlotSnapshot, state: &DeviceState) -> bool {
    let mut expected = live.transport.clone();
    expected.device_features_sel = 0;
    expected.driver_features_sel = 0;
    for queue in &mut expected.queues {
        queue.activated = queue.ready || queue.next_avail != 0 || queue.next_used != 0;
    }
    live.slot == slot && transport(state) == expected
}

/// Checks the certified device-specific fields against a freshly constructed device and
/// returns the record that device expects to be restored with.
///
/// # Errors
///
/// Returns [`SnapshotError::DeviceStateNotCanonical`] when the fresh backend does not match
/// the certified geometry, identity, or link state.
pub(super) fn fresh_record(
    slot: Slot,
    devices: &BusDevices,
    state: &DeviceState,
) -> Result<Vec<u8>, SnapshotError> {
    let mismatch = || SnapshotError::DeviceStateNotCanonical(slot);
    let blob = match (state.kind(), state.specific()) {
        (DeviceKind::RootBlock, DeviceSpecific::Block(block)) => {
            let device = &devices.root;
            if device.capacity_sectors() != block.capacity_sectors
                || device.blk_size() != block.block_size
            {
                return Err(mismatch());
            }
            device.snapshot_state()
        }
        (DeviceKind::OverlayBlock, DeviceSpecific::Block(block)) => {
            let device = devices.overlay.as_ref().ok_or_else(mismatch)?;
            if device.capacity_sectors() != block.capacity_sectors
                || device.blk_size() != block.block_size
            {
                return Err(mismatch());
            }
            device.snapshot_state()
        }
        (DeviceKind::Net, DeviceSpecific::Net { mac, link_up }) => {
            let device = devices.net.as_ref().ok_or_else(mismatch)?;
            if device.mac() != mac || device.link_up() != link_up || link_up {
                return Err(mismatch());
            }
            device.snapshot_state()
        }
        (DeviceKind::Vsock, DeviceSpecific::Vsock { cid_placeholder }) => {
            if devices.vsock.guest_cid() != cid_placeholder {
                return Err(mismatch());
            }
            devices.vsock.snapshot_state()
        }
        (DeviceKind::Rng, DeviceSpecific::Rng) => devices.rng.snapshot_state(),
        _ => return Err(mismatch()),
    };
    Ok(blob)
}
