//! Construction of the declared device models behind the shared bus, and the bus handle itself.
//!
//! The immutable root is a preopened file handed in by the caller, the private overlay is
//! another when the Generation declared writable storage, the network device sits behind the
//! link-down loopback placeholder because this host has no TAP broker yet, the vsock device
//! carries the assigned guest CID, and entropy comes from a fresh `/dev/urandom` handle.
//! Nothing here registers with KVM.
//!
//! The set of devices is the Generation's, not a constant: a machine that declared no writable
//! storage gets no overlay device and a machine that declared no egress gets no network device.
//! The caller states the set it means and hands over resources for exactly it, and a
//! disagreement between the two is refused here rather than resolved by preferring one.

use std::{
    fs::File,
    sync::{Condvar, Mutex, MutexGuard, PoisonError},
};

use super::error::{MachineError, MachineErrorKind, Phase};
use crate::virtio::{
    BLOCK_SERIAL_LEN, BlockDevice, BlockRole, BusDevices, Detached, DeviceSet, FileBackend,
    LoopbackBackend, MmioBus, NetDevice, OsEntropy, RngDevice, VsockDevice,
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

/// Binds the declared device models to fresh transports.
pub(crate) fn build_bus(
    disks: SandboxDisks,
    identity: DeviceIdentity,
    devices: DeviceSet,
) -> Result<MmioBus, MachineError> {
    MmioBus::new(build_devices(disks, identity, devices)?)
        .map_err(|error| MachineError::new(Phase::Devices, MachineErrorKind::Bus(error)))
}

/// Constructs the declared device models with fresh backends and no transport binding.
///
/// Snapshot restore needs the unbound models so it can rebuild every transport from captured
/// state instead of from the power-on defaults `build_bus` installs.
pub(crate) fn build_devices(
    disks: SandboxDisks,
    identity: DeviceIdentity,
    devices: DeviceSet,
) -> Result<BusDevices, MachineError> {
    let overlay = match (devices.overlay(), disks.overlay) {
        (true, Some(file)) => Some(block(
            BlockRole::PrivateOverlay,
            file,
            false,
            OVERLAY_SERIAL,
        )?),
        (false, None) => None,
        _ => return Err(overlay_disagreement()),
    };
    build_around_overlay(disks.root, overlay, identity, devices)
}

/// Builds the declared device models with the private overlay declared rather than held.
///
/// A prepared worker is built before the Instance it will serve exists, and the prepared worker
/// protocol forbids it from holding a private disk head until it is claimed. The overlay device
/// still has to exist by then, because having built it is most of what preparing a worker in
/// advance buys, so it is built against the capacity the head will have and the head itself is
/// attached at claim. A machine with no overlay at all declares no capacity and gets no device.
///
/// # Errors
///
/// Returns the typed device failure, exactly as [`build_devices`] does.
pub(crate) fn build_devices_detached_overlay(
    root: File,
    overlay_capacity_bytes: Option<u64>,
    identity: DeviceIdentity,
    devices: DeviceSet,
) -> Result<BusDevices, MachineError> {
    let overlay = match (devices.overlay(), overlay_capacity_bytes) {
        (true, Some(capacity)) => Some(
            BlockDevice::new(
                BlockRole::PrivateOverlay,
                Box::new(Detached::new(capacity, false)),
                BLOCK_SIZE,
                serial(OVERLAY_SERIAL),
            )
            .map_err(|error| MachineError::new(Phase::Devices, MachineErrorKind::Block(error)))?,
        ),
        (false, None) => None,
        _ => return Err(overlay_disagreement()),
    };
    build_around_overlay(root, overlay, identity, devices)
}

/// A caller that names a device set and hands over resources for a different one has two
/// different ideas about what the machine is, and picking either would silently launch the
/// Generation as something it is not.
fn overlay_disagreement() -> MachineError {
    MachineError::invalid(Phase::Devices, "the device set and the overlay disagree")
}

/// The devices that are the same however the overlay was obtained.
fn build_around_overlay(
    root: File,
    overlay: Option<BlockDevice>,
    identity: DeviceIdentity,
    devices: DeviceSet,
) -> Result<BusDevices, MachineError> {
    let root = block(BlockRole::ImmutableRoot, root, true, ROOT_SERIAL)?;
    let net = devices
        .net()
        .then(|| NetDevice::new(Box::new(LoopbackBackend::default()), identity.guest_mac));
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

    fn full_disks() -> SandboxDisks {
        SandboxDisks {
            root: image(64 * 4096),
            overlay: Some(image(64 * 1024 * 1024)),
        }
    }

    #[test]
    fn builds_the_five_slots_with_fixed_geometry_and_identity() {
        let bus = build_bus(full_disks(), identity(), DeviceSet::FULL).unwrap();
        assert_eq!(bus.root().device().role(), BlockRole::ImmutableRoot);
        assert_eq!(bus.root().device().blk_size(), BLOCK_SIZE);
        assert_eq!(bus.root().device().capacity_sectors(), 64 * 8);
        assert_ne!(bus.root().device().feature_allowlist_ro(), 0);
        assert_eq!(
            bus.overlay().unwrap().device().role(),
            BlockRole::PrivateOverlay
        );
        assert_eq!(bus.vsock().device().guest_cid(), 3);
        assert_eq!(bus.net().unwrap().device().mac(), identity().guest_mac);
        assert!(!bus.net().unwrap().device().link_up());
        assert_eq!(Slot::ALL.len(), 5);
    }

    #[test]
    fn builds_only_the_declared_devices() {
        let disks = SandboxDisks {
            root: image(64 * 4096),
            overlay: None,
        };
        let bus = build_bus(disks, identity(), DeviceSet::new(false, false)).unwrap();
        assert!(bus.overlay().is_none());
        assert!(bus.net().is_none());
        assert_eq!(bus.device_set(), DeviceSet::new(false, false));
        assert_eq!(bus.device_set().present().count(), 3);
    }

    #[test]
    fn refuses_a_head_the_declared_set_has_no_slot_for() {
        let Err(error) = build_bus(full_disks(), identity(), DeviceSet::new(false, true)) else {
            panic!("an overlay handed to a machine with no overlay slot must be refused");
        };
        assert_eq!(error.phase(), Phase::Devices);
        let disks = SandboxDisks {
            root: image(64 * 4096),
            overlay: None,
        };
        let Err(error) = build_bus(disks, identity(), DeviceSet::FULL) else {
            panic!("a declared overlay with no head must be refused");
        };
        assert_eq!(error.phase(), Phase::Devices);
    }

    #[test]
    fn rejects_a_reserved_cid_and_an_unaligned_root() {
        let Err(error) = build_bus(
            full_disks(),
            DeviceIdentity {
                guest_cid: 2,
                ..identity()
            },
            DeviceSet::FULL,
        ) else {
            panic!("a reserved CID must be rejected");
        };
        assert_eq!(error.phase(), Phase::Devices);
        assert!(matches!(error.kind(), MachineErrorKind::Vsock(_)));
        let disks = SandboxDisks {
            root: image(4096 + 512),
            overlay: Some(image(64 * 1024 * 1024)),
        };
        let Err(error) = build_bus(disks, identity(), DeviceSet::FULL) else {
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
