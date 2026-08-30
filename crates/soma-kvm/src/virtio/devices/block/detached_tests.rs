//! Building a block device against a declared shape and attaching its store later.

use super::backend::{Detached, MemoryBackend};
use super::tests::SERIAL;
use super::{BlockConfigError, BlockDevice, BlockRole};
use crate::virtio::devices::block::backend::{BackendError, BlockBackend};

const SECTORS: usize = 8;
const CAPACITY: u64 = SECTORS as u64 * 512;

fn declared() -> BlockDevice {
    BlockDevice::new(
        BlockRole::PrivateOverlay,
        Box::new(Detached::new(CAPACITY, false)),
        512,
        SERIAL,
    )
    .expect("a device may be built against a declared shape")
}

/// A worker built without a head must refuse every access rather than invent one.
#[test]
fn a_detached_store_serves_no_byte() {
    let mut backend = Detached::new(CAPACITY, false);
    assert_eq!(backend.capacity_bytes(), CAPACITY);
    assert_eq!(
        backend.read_at(0, &mut [0; 512]),
        Err(BackendError::OutOfRange)
    );
    assert_eq!(
        backend.write_at(0, &[0; 512]),
        Err(BackendError::OutOfRange)
    );
    assert_eq!(backend.flush(), Err(BackendError::OutOfRange));
}

/// The head a claim delivers replaces the declaration it was measured against.
#[test]
fn a_head_of_the_declared_shape_attaches() {
    let mut device = declared();
    assert_eq!(
        device.attach(Box::new(MemoryBackend::zeroed(SECTORS, false))),
        Ok(())
    );
}

/// The guest has already been told a capacity, so a head of another size cannot be substituted.
#[test]
fn a_head_of_another_capacity_is_refused() {
    let mut device = declared();
    assert_eq!(
        device.attach(Box::new(MemoryBackend::zeroed(SECTORS * 2, false))),
        Err(BlockConfigError::AttachedShapeDiffers)
    );
}

/// A read-only store cannot become the private head, whatever its size.
#[test]
fn a_read_only_head_is_refused_for_the_private_overlay() {
    let mut device = declared();
    assert_eq!(
        device.attach(Box::new(MemoryBackend::zeroed(SECTORS, true))),
        Err(BlockConfigError::RoleMismatch)
    );
}
