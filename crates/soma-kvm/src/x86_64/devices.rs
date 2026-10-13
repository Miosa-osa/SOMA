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
use crate::contract::MachineContract;
use crate::virtio::{
    BlockDevice, BlockRole, BusDevices, Detached, DeviceSet, LoopbackBackend, MemDevice, MmioBus,
    NetDevice, OsEntropy, RegionBacking, RngDevice, VsockDevice,
};

mod shape;

/// The hotplug region a machine placed, as `(guest address, bytes, host backing)`, or `None` for
/// a machine with no memory device.
pub(crate) type Hotplug = Option<(u64, u64, Box<dyn RegionBacking + Send>)>;

pub(crate) use shape::block_transfer;
pub use shape::{BLOCK_SIZE, DeviceIdentity, SandboxDisks};
use shape::{OVERLAY_SERIAL, ROOT_SERIAL, VOLUME_SERIAL, block, serial};

/// The bus shared between the vCPU thread, the device thread, and the control channel.
///
/// The condition variable is signalled after every device-thread pass and by the vCPU thread
/// when it stops, so a control-channel waiter never sleeps past a state change.
/// The optional block devices, grouped so a builder takes one argument for both.
struct OptionalBlocks {
    overlay: Option<BlockDevice>,
    volume: Option<BlockDevice>,
}

/// The capacity each optional block device is declared with, when it is declared at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DetachedCapacities {
    /// The private head's capacity, or `None` for a Generation with no writable storage.
    pub(crate) overlay: Option<u64>,
    /// The durable volume's capacity, or `None` for a Generation with no persistence.
    pub(crate) volume: Option<u64>,
}

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

/// Binds the declared device models to fresh transports.
pub(crate) fn build_bus(
    disks: SandboxDisks,
    identity: DeviceIdentity,
    devices: DeviceSet,
    contract: MachineContract,
    hotplug: Hotplug,
) -> Result<MmioBus, MachineError> {
    MmioBus::new(build_devices(disks, identity, devices, contract, hotplug)?)
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
    contract: MachineContract,
    hotplug: Hotplug,
) -> Result<BusDevices, MachineError> {
    let overlay = match (devices.overlay(), disks.overlay) {
        (true, Some(file)) => Some(block(
            BlockRole::PrivateOverlay,
            file,
            false,
            OVERLAY_SERIAL,
            block_transfer(contract),
        )?),
        (false, None) => None,
        _ => return Err(overlay_disagreement()),
    };
    let volume = match (devices.volume(), disks.volume) {
        (true, Some(file)) => Some(
            block(
                BlockRole::DurableVolume,
                file,
                false,
                VOLUME_SERIAL,
                block_transfer(contract),
            )?
            // A cold boot is given the file the machine will write, so its volume device serves
            // that file rather than standing on a placeholder: there is no swap to wait for, and
            // a second store must not replace what the guest is already writing.
            .into_serving(),
        ),
        (false, None) => None,
        _ => return Err(volume_disagreement()),
    };
    build_around_overlay(
        disks.root,
        OptionalBlocks { overlay, volume },
        identity,
        devices,
        contract,
        hotplug,
    )
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
    capacities: DetachedCapacities,
    identity: DeviceIdentity,
    devices: DeviceSet,
    contract: MachineContract,
    hotplug: Hotplug,
) -> Result<BusDevices, MachineError> {
    let DetachedCapacities {
        overlay: overlay_capacity_bytes,
        volume: volume_capacity_bytes,
    } = capacities;
    let overlay = match (devices.overlay(), overlay_capacity_bytes) {
        (true, Some(capacity)) => Some(
            BlockDevice::new(
                BlockRole::PrivateOverlay,
                Box::new(Detached::new(capacity, false)),
                BLOCK_SIZE,
                serial(OVERLAY_SERIAL),
                block_transfer(contract),
            )
            .map_err(|error| MachineError::new(Phase::Devices, MachineErrorKind::Block(error)))?,
        ),
        (false, None) => None,
        _ => return Err(overlay_disagreement()),
    };
    // The volume device is built exactly as the overlay is: declared against the capacity it
    // will have, with the backing attached later. ADR 0053 requires the volume to be present
    // in the captured device state backed by a placeholder, so the restore reproduces this
    // device and the launch binds the Instance's own file over it.
    let volume = match (devices.volume(), volume_capacity_bytes) {
        (true, Some(capacity)) => Some(
            BlockDevice::new(
                BlockRole::DurableVolume,
                Box::new(Detached::new(capacity, false)),
                BLOCK_SIZE,
                serial(VOLUME_SERIAL),
                block_transfer(contract),
            )
            .map_err(|error| MachineError::new(Phase::Devices, MachineErrorKind::Block(error)))?,
        ),
        (false, None) => None,
        _ => return Err(volume_disagreement()),
    };
    build_around_overlay(
        root,
        OptionalBlocks { overlay, volume },
        identity,
        devices,
        contract,
        hotplug,
    )
}

/// A caller that names a device set and hands over resources for a different one has two
/// different ideas about what the machine is, and picking either would silently launch the
/// Generation as something it is not.
fn overlay_disagreement() -> MachineError {
    MachineError::invalid(Phase::Devices, "the device set and the overlay disagree")
}

/// The same disagreement for the durable volume: a caller that names a device set with a
/// volume and hands over no resource for one has two different ideas about the machine.
fn volume_disagreement() -> MachineError {
    MachineError::invalid(Phase::Devices, "the device set and the volume disagree")
}

/// The devices that are the same however the overlay was obtained.
///
/// The memory device is built from the hotplug region the machine placed, its guest address and
/// size, and the backing over the host memory behind it, because the region's placement belongs
/// to the machine and not to the device table.
fn build_around_overlay(
    root: File,
    blocks: OptionalBlocks,
    identity: DeviceIdentity,
    devices: DeviceSet,
    contract: MachineContract,
    hotplug: Hotplug,
) -> Result<BusDevices, MachineError> {
    if devices.mem() != hotplug.is_some() {
        return Err(MachineError::invalid(
            Phase::Devices,
            "the device set and the hotplug region disagree",
        ));
    }
    let mem = hotplug
        .map(|(addr, size, backing)| {
            MemDevice::new(addr, size, backing).map_err(|_| {
                MachineError::invalid(
                    Phase::Devices,
                    "the hotplug region is not a valid memory device",
                )
            })
        })
        .transpose()?;
    let OptionalBlocks { overlay, volume } = blocks;
    let root = block(
        BlockRole::ImmutableRoot,
        root,
        true,
        ROOT_SERIAL,
        block_transfer(contract),
    )?;
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
        volume,
        net,
        vsock,
        rng,
        mem,
    })
}

#[cfg(test)]
mod tests;
