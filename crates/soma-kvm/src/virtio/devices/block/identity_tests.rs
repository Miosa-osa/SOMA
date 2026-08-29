//! Constructor, configuration-space, and snapshot identity tests.

use super::backend::MemoryBackend;
use super::state::{BLOCK_STATE_LEN, BlockState};
use super::tests::{SERIAL, boot, device};
use super::*;
use crate::virtio::transport::MmioTransport;
use crate::virtio::transport::registers::AccessWidth;

#[test]
fn constructor_rejects_bad_role_block_size_and_capacity() {
    let ro = || Box::new(MemoryBackend::zeroed(8, true));
    assert_eq!(
        BlockDevice::new(BlockRole::PrivateOverlay, ro(), 512, SERIAL).err(),
        Some(BlockConfigError::RoleMismatch)
    );
    assert!(
        BlockDevice::new(BlockRole::ImmutableRoot, ro(), 4096, SERIAL).is_ok(),
        "8 sectors is exactly one 4 KiB block"
    );
    assert_eq!(
        BlockDevice::new(BlockRole::ImmutableRoot, ro(), 1000, SERIAL).err(),
        Some(BlockConfigError::InvalidBlockSize { blk_size: 1000 })
    );
    assert_eq!(
        BlockDevice::new(
            BlockRole::ImmutableRoot,
            Box::new(MemoryBackend::zeroed(0, true)),
            512,
            SERIAL
        )
        .err(),
        Some(BlockConfigError::InvalidCapacity { capacity_bytes: 0 })
    );
}

#[test]
fn config_space_exposes_capacity_and_block_size_read_only() {
    let (_, mut t) = boot(BlockRole::PrivateOverlay, 8);
    let cfg = |t: &mut MmioTransport<BlockDevice>, off: u64, w: AccessWidth| {
        t.read(0x100 + off, w).expect("cfg")
    };
    assert_eq!(cfg(&mut t, 0, AccessWidth::U64), 8);
    assert_eq!(cfg(&mut t, 20, AccessWidth::U32), 512);
    let mem = crate::virtio::guest_memory::VecGuestMemory::flat(16).expect("mem");
    assert!(t.write(0x100, AccessWidth::U32, 1, &mem).is_err());
    assert!(t.read(0x100 + 24, AccessWidth::U8).is_err());
}

#[test]
fn snapshot_state_round_trips_and_rejects_mismatches() {
    let (_, t) = boot(BlockRole::PrivateOverlay, 8);
    let raw = t.device().snapshot_state();
    assert_eq!(raw.len(), BLOCK_STATE_LEN);
    let state = BlockState::from_bytes(&raw).expect("decode");
    assert_eq!(state.to_bytes().to_vec(), raw);
    let mut fresh = device(BlockRole::PrivateOverlay, 8);
    assert_eq!(fresh.restore_state(&raw), Ok(()));
    assert_eq!(
        device(BlockRole::ImmutableRoot, 8).restore_state(&raw),
        Err(DeviceStateError::Incompatible)
    );
    assert_eq!(
        device(BlockRole::PrivateOverlay, 16).restore_state(&raw),
        Err(DeviceStateError::Incompatible)
    );
    assert_eq!(
        fresh.restore_state(&raw[..raw.len() - 1]),
        Err(DeviceStateError::Malformed)
    );
    let mut wrong_id = raw.clone();
    wrong_id[1] = 1;
    assert_eq!(
        fresh.restore_state(&wrong_id),
        Err(DeviceStateError::Incompatible)
    );
    let mut wrong_features = raw.clone();
    wrong_features[5] ^= 0x20;
    assert_eq!(
        fresh.restore_state(&wrong_features),
        Err(DeviceStateError::Incompatible)
    );
    let mut wrong_version = raw.clone();
    wrong_version[0] = 2;
    assert_eq!(
        fresh.restore_state(&wrong_version),
        Err(DeviceStateError::Malformed)
    );
    let mut bad_role = raw;
    bad_role[13] = 7;
    assert_eq!(
        fresh.restore_state(&bad_role),
        Err(DeviceStateError::Malformed)
    );
}
