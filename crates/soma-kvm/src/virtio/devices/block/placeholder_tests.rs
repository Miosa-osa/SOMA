//! The durable volume's placeholder: born on it, swapped exactly once, never left behind.

use super::backend::{BlockBackend, MemoryBackend};
use super::*;

const SERIAL: [u8; BLOCK_SERIAL_LEN] = *b"soma-volume-test-001";

fn built(role: BlockRole, sectors: usize) -> BlockDevice {
    BlockDevice::new(
        role,
        Box::new(MemoryBackend::zeroed(
            sectors,
            role == BlockRole::ImmutableRoot,
        )),
        512,
        SERIAL,
        TransferShape::Undeclared,
    )
    .expect("device")
}

fn store(sectors: usize) -> Box<dyn BlockBackend + Send> {
    Box::new(MemoryBackend::zeroed(sectors, false))
}

#[test]
fn a_durable_volume_is_born_on_its_placeholder_and_no_other_role_is() {
    let volume = built(BlockRole::DurableVolume, 8);
    assert!(volume.is_placeholder());
    // The capacity the guest is told is the Instance's own and not the placeholder's: the file is
    // bound over the placeholder before the guest reads the device at all.
    assert_eq!(volume.capacity_sectors(), 8);
    assert_eq!(SECTOR_SIZE, 512);

    assert!(!built(BlockRole::PrivateOverlay, 8).is_placeholder());
    assert!(!built(BlockRole::ImmutableRoot, 8).is_placeholder());
}

#[test]
fn a_volume_takes_exactly_one_swap() {
    let mut volume = built(BlockRole::DurableVolume, 8);
    volume.attach(store(8)).expect("the launch's one swap");
    assert!(!volume.is_placeholder());

    // A second swap would replace a store a running guest may already have written to.
    assert_eq!(
        volume.attach(store(8)).err(),
        Some(BlockConfigError::AlreadyAttached)
    );
}

#[test]
fn a_refused_swap_leaves_the_placeholder_in_place() {
    let mut volume = built(BlockRole::DurableVolume, 8);
    assert_eq!(
        volume.attach(store(16)).err(),
        Some(BlockConfigError::AttachedShapeDiffers)
    );
    assert!(volume.is_placeholder());
    // The device is still the one it was, so the right store still lands.
    volume.attach(store(8)).expect("the right shape");
    assert!(!volume.is_placeholder());
}

#[test]
fn the_overlay_keeps_the_attachment_behaviour_it_had() {
    let mut overlay = built(BlockRole::PrivateOverlay, 8);
    assert!(!overlay.is_placeholder());
    overlay.attach(store(8)).expect("the private head");
    assert!(!overlay.is_placeholder());
    overlay
        .attach(store(8))
        .expect("the overlay path is unchanged");
}
