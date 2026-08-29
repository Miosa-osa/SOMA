//! Construction of the five device models behind the shared bus, and the bus handle itself.
//!
//! The immutable root and the private overlay are preopened files handed in by the caller,
//! the network device sits behind the link-down loopback placeholder because this host has no
//! TAP broker yet, the vsock device carries the assigned guest CID, and entropy comes from a
//! fresh `/dev/urandom` handle. Nothing here registers with KVM.

use std::{
    fs::File,
    sync::{Condvar, Mutex, MutexGuard, PoisonError},
};

use super::error::{MachineError, MachineErrorKind, Phase};
use crate::virtio::{
    BLOCK_SERIAL_LEN, BlockDevice, BlockRole, BusDevices, FileBackend, LoopbackBackend, MmioBus,
    NetDevice, OsEntropy, RngDevice, VsockDevice,
};

/// Logical block size reported by both block devices; equal to the EROFS and ext4 block size.
pub const BLOCK_SIZE: u32 = 4096;
const ROOT_SERIAL: &[u8] = b"soma-root";
const OVERLAY_SERIAL: &[u8] = b"soma-overlay";

/// The bus shared between the vCPU thread, the device thread, and the control channel.
///
/// The condition variable is signalled after every device-thread pass and by the vCPU thread
/// when it stops, so a control-channel waiter never sleeps past a state change.
pub(crate) struct SharedBus {
    bus: Mutex<MmioBus>,
    changed: Condvar,
}

impl SharedBus {
    pub(crate) fn new(bus: MmioBus) -> Self {
        Self {
            bus: Mutex::new(bus),
            changed: Condvar::new(),
        }
    }

    /// Locks the bus; a poisoned lock is recovered because every state it guards is bounded
    /// and the guest is about to be torn down anyway.
    pub(crate) fn lock(&self) -> MutexGuard<'_, MmioBus> {
        self.bus.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) const fn changed(&self) -> &Condvar {
        &self.changed
    }

    pub(crate) fn notify_all(&self) {
        self.changed.notify_all();
    }
}

/// Preopened disk images: the immutable root must not be writable through this handle.
pub struct SandboxDisks {
    /// The EROFS Generation root, opened read-only.
    pub root: File,
    /// The Instance-private ext4 overlay head, opened read-write.
    pub overlay: File,
}

/// Non-secret device identity assigned to one Instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceIdentity {
    /// The guest vsock context identifier, at least 3.
    pub guest_cid: u32,
    /// The effective unicast MAC the guest will install; reported in virtio-net config.
    pub guest_mac: [u8; 6],
}

fn serial(name: &[u8]) -> [u8; BLOCK_SERIAL_LEN] {
    let mut serial = [0_u8; BLOCK_SERIAL_LEN];
    serial[..name.len()].copy_from_slice(name);
    serial
}

fn block(
    role: BlockRole,
    file: File,
    read_only: bool,
    name: &[u8],
) -> Result<BlockDevice, MachineError> {
    let backend = FileBackend::new(file, read_only)
        .map_err(|error| MachineError::io(Phase::Devices, &error))?;
    BlockDevice::new(role, Box::new(backend), BLOCK_SIZE, serial(name))
        .map_err(|error| MachineError::new(Phase::Devices, MachineErrorKind::Block(error)))
}

/// Binds the five device models to fresh transports.
pub(crate) fn build_bus(
    disks: SandboxDisks,
    identity: DeviceIdentity,
) -> Result<MmioBus, MachineError> {
    MmioBus::new(build_devices(disks, identity)?)
        .map_err(|error| MachineError::new(Phase::Devices, MachineErrorKind::Bus(error)))
}

/// Constructs the five device models with fresh backends and no transport binding.
///
/// Snapshot restore needs the unbound models so it can rebuild every transport from captured
/// state instead of from the power-on defaults `build_bus` installs.
pub(crate) fn build_devices(
    disks: SandboxDisks,
    identity: DeviceIdentity,
) -> Result<BusDevices, MachineError> {
    let root = block(BlockRole::ImmutableRoot, disks.root, true, ROOT_SERIAL)?;
    let overlay = block(
        BlockRole::PrivateOverlay,
        disks.overlay,
        false,
        OVERLAY_SERIAL,
    )?;
    let net = NetDevice::new(Box::new(LoopbackBackend::default()), identity.guest_mac);
    let vsock = VsockDevice::new(u64::from(identity.guest_cid))
        .map_err(|error| MachineError::new(Phase::Devices, MachineErrorKind::Vsock(error)))?;
    let entropy = OsEntropy::open()
        .map_err(|error| MachineError::new(Phase::Devices, MachineErrorKind::Entropy(error)))?;
    let rng = RngDevice::new(Box::new(entropy));
    Ok(BusDevices {
        root,
        overlay,
        net,
        vsock,
        rng,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtio::{Slot, VIRTIO_BLK_F_RO};

    fn image(bytes: u64) -> File {
        let file = tempfile::tempfile().unwrap();
        file.set_len(bytes).unwrap();
        file
    }

    fn identity() -> DeviceIdentity {
        DeviceIdentity {
            guest_cid: 3,
            guest_mac: [0x02, 0x53, 0x4f, 0x4d, 0x41, 0x01],
        }
    }

    #[test]
    fn builds_the_five_slots_with_fixed_geometry_and_identity() {
        let disks = SandboxDisks {
            root: image(64 * 4096),
            overlay: image(64 * 1024 * 1024),
        };
        let bus = build_bus(disks, identity()).unwrap();
        assert_eq!(bus.root().device().role(), BlockRole::ImmutableRoot);
        assert_eq!(bus.root().device().blk_size(), BLOCK_SIZE);
        assert_eq!(bus.root().device().capacity_sectors(), 64 * 8);
        assert_ne!(bus.root().device().feature_allowlist_ro(), 0);
        assert_eq!(bus.overlay().device().role(), BlockRole::PrivateOverlay);
        assert_eq!(bus.vsock().device().guest_cid(), 3);
        assert_eq!(bus.net().device().mac(), identity().guest_mac);
        assert!(!bus.net().device().link_up());
        assert_eq!(Slot::ALL.len(), 5);
    }

    #[test]
    fn rejects_a_reserved_cid_and_an_unaligned_root() {
        let disks = SandboxDisks {
            root: image(4096),
            overlay: image(64 * 1024 * 1024),
        };
        let Err(error) = build_bus(
            disks,
            DeviceIdentity {
                guest_cid: 2,
                ..identity()
            },
        ) else {
            panic!("a reserved CID must be rejected");
        };
        assert_eq!(error.phase(), Phase::Devices);
        assert!(matches!(error.kind(), MachineErrorKind::Vsock(_)));
        let disks = SandboxDisks {
            root: image(4096 + 512),
            overlay: image(64 * 1024 * 1024),
        };
        let Err(error) = build_bus(disks, identity()) else {
            panic!("an unaligned root must be rejected");
        };
        assert!(matches!(error.kind(), MachineErrorKind::Block(_)));
    }

    impl BlockDevice {
        fn feature_allowlist_ro(&self) -> u64 {
            self.role().features() & VIRTIO_BLK_F_RO
        }
    }
}
