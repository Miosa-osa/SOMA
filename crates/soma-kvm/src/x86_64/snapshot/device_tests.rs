//! The capture path refuses to publish a durable volume that is not on its placeholder.

use super::{SnapshotError, device};
use crate::contract::MachineContract;
use crate::snapshot::Digest;
use crate::virtio::devices::block::backend::MemoryBackend;
use crate::virtio::devices::rng::backend::CounterEntropy;
use crate::virtio::{
    BlockDevice, BlockRole, BusDevices, MmioBus, RngDevice, Slot, TransferShape, VsockDevice,
};

fn block(role: BlockRole, sectors: usize) -> BlockDevice {
    BlockDevice::new(
        role,
        Box::new(MemoryBackend::zeroed(
            sectors,
            role == BlockRole::ImmutableRoot,
        )),
        512,
        [0; 20],
        TransferShape::Undeclared,
    )
    .expect("block")
}

fn bus() -> MmioBus {
    MmioBus::new(BusDevices {
        root: block(BlockRole::ImmutableRoot, 8),
        overlay: None,
        volume: Some(block(BlockRole::DurableVolume, 8)),
        net: None,
        vsock: VsockDevice::new(3).expect("vsock"),
        rng: RngDevice::new(Box::new(CounterEntropy::default())),
        mem: None,
    })
    .expect("bus")
}

fn volume_state(bus: &mut MmioBus, image: Digest) -> Result<(), SnapshotError> {
    let record = |record: &crate::virtio::SlotSnapshot| matches!(record.slot, Slot::Volume);
    let live = bus
        .snapshot_all()
        .into_iter()
        .find(record)
        .expect("the volume record");
    let specific = device::specific(bus, Slot::Volume, image).expect("specific");
    device::canonical(bus, Slot::Volume, &live, specific, MachineContract::V1).map(|_| ())
}

#[test]
fn a_capture_publishes_a_volume_only_while_it_is_a_placeholder() {
    let image = Digest::from_bytes([0; 32]);
    let mut bus = bus();
    // A freshly built machine carries the placeholder the Generation declared, which is the
    // device state a capture is allowed to read.
    assert!(bus.volume().expect("volume slot").device().is_placeholder());

    // The launch binds the Instance's own file over the placeholder before the guest runs, so a
    // machine that reached a capture in that state has tenant data behind a device whose
    // manifest would describe an empty disk. The refusal comes before every other check, so it
    // holds whatever the driver negotiated.
    bus.volume_mut()
        .expect("volume slot")
        .device_mut()
        .attach(Box::new(MemoryBackend::zeroed(8, false)))
        .expect("the launch's one swap");
    assert!(matches!(
        volume_state(&mut bus, image),
        Err(SnapshotError::VolumeNotPlaceholder)
    ));
}
