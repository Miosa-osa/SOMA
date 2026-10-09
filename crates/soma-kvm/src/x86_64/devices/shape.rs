//! What the devices of one machine are made of, before any of them exists.
//!
//! The resources a caller has to open, the identity a caller has to assign, and the two
//! translations from the declared shape to the device models: a machine contract to the
//! transfer shape its block devices advertise, and a role plus a store to a built device.

use std::fs::File;

use super::super::error::{MachineError, MachineErrorKind, Phase};
use crate::contract::MachineContract;
use crate::virtio::{BLOCK_SERIAL_LEN, BlockDevice, BlockRole, FileBackend, TransferShape};

/// Logical block size reported by both block devices; equal to the EROFS and ext4 block size.
pub const BLOCK_SIZE: u32 = 4096;
pub(super) const ROOT_SERIAL: &[u8] = b"soma-root";
pub(super) const OVERLAY_SERIAL: &[u8] = b"soma-overlay";

/// The transfer shape the block devices of a machine contract advertise.
///
/// Version 1 has offered this device surface since it was certified and its digest is pinned to
/// it, so version 1 keeps advertising nothing and keeps its kernel's own defaults. Version 2 is
/// the shape a build workload runs in, and there the device states the largest request it
/// answers so the driver never forms one the parser has to reject.
#[must_use]
pub(crate) const fn block_transfer(contract: MachineContract) -> TransferShape {
    match contract {
        MachineContract::V1 => TransferShape::Undeclared,
        MachineContract::V2 => TransferShape::Declared,
    }
}

/// Preopened disk images: the immutable root must not be writable through this handle.
pub struct SandboxDisks {
    /// The EROFS Generation root, opened read-only.
    pub root: File,
    /// The Instance-private ext4 overlay head, opened read-write, when there is one.
    ///
    /// A Generation that declared no writable storage has none: the guest mounts the immutable
    /// root read-only and never composes an `OverlayFS`, so there is no head to clone and the
    /// largest and most variable cost on the launch path is not paid at all.
    pub overlay: Option<File>,
}

/// Non-secret device identity assigned to one Instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceIdentity {
    /// The guest vsock context identifier, at least 3.
    pub guest_cid: u32,
    /// The effective unicast MAC the guest will install; reported in virtio-net config.
    pub guest_mac: [u8; 6],
}

/// Pads a short name into the fixed-width serial field the guest reads for `GET_ID`.
pub(super) fn serial(name: &[u8]) -> [u8; BLOCK_SERIAL_LEN] {
    let mut serial = [0_u8; BLOCK_SERIAL_LEN];
    serial[..name.len()].copy_from_slice(name);
    serial
}

/// Builds one block device over an already-open store.
pub(super) fn block(
    role: BlockRole,
    file: File,
    read_only: bool,
    name: &[u8],
    transfer: TransferShape,
) -> Result<BlockDevice, MachineError> {
    let backend = FileBackend::new(file, read_only)
        .map_err(|error| MachineError::io(Phase::Devices, &error))?;
    BlockDevice::new(role, Box::new(backend), BLOCK_SIZE, serial(name), transfer)
        .map_err(|error| MachineError::new(Phase::Devices, MachineErrorKind::Block(error)))
}
