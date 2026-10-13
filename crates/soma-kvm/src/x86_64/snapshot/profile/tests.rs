//! The host profile's own tests: the device contract, the per-slot expectations and the CPU template.

use super::*;
use crate::snapshot::device_state::DeviceKind;
use crate::virtio::{
    BLOCK_QUEUE_MAX, Slot, TransferShape, VIRTIO_BLK_F_SEG_MAX, VIRTIO_BLK_F_SIZE_MAX,
};

/// The device surface of machine contract version 1, as a digest that must never move.
///
/// Version 1 is certified and served: every snapshot taken under it names this value in its
/// manifest header, and a restore compares it before it maps a byte of memory. Changing the
/// surface changes this digest and turns every one of those snapshots into a refusal, so the
/// value is pinned here rather than left to a round trip that would pass either way.
const V1_DEVICE_SURFACE: &str = "d3d2be87c258c5cb40469fdbe60b82c579821ba265360d79afec9abef88580d3";

#[test]
fn the_version_one_device_surface_is_the_certified_one() {
    assert_eq!(
        device_contract(DeviceSet::FULL, MachineContract::V1).to_string(),
        V1_DEVICE_SURFACE,
        "the version 1 device surface moved, and every version 1 snapshot names the old one"
    );
}

#[test]
fn a_version_two_block_slot_declares_its_transfer_limits_and_version_one_does_not() {
    // The two contracts are different devices, so a snapshot of one is refused by the other.
    assert_ne!(
        device_contract(DeviceSet::FULL, MachineContract::V1),
        device_contract(DeviceSet::FULL, MachineContract::V2)
    );
    let v1 = expectation(Slot::Overlay, MachineContract::V1);
    let v2 = expectation(Slot::Overlay, MachineContract::V2);
    let declared = VIRTIO_BLK_F_SIZE_MAX | VIRTIO_BLK_F_SEG_MAX;
    assert_eq!(v2.kind, DeviceKind::OverlayBlock);
    assert_eq!(v2.negotiated_features & declared, declared);
    assert_eq!(v1.negotiated_features & declared, 0);
    assert_eq!(v1.queue_limits, v2.queue_limits);
    assert_eq!(v1.queue_limits[0], BLOCK_QUEUE_MAX[0]);
    // The one thing the two contracts share is the role's own allowlist underneath.
    assert_eq!(
        BlockRole::PrivateOverlay.features(TransferShape::Declared)
            ^ BlockRole::PrivateOverlay.features(TransferShape::Undeclared),
        declared
    );
}
