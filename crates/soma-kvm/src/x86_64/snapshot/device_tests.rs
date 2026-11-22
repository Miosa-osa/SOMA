//! The capture path refuses to publish a durable volume that is not on its placeholder.

use super::{SnapshotError, capture::require_placeholder_volume};
use crate::virtio::devices::block::backend::MemoryBackend;
use crate::virtio::devices::rng::backend::CounterEntropy;
use crate::virtio::{
    BlockDevice, BlockRole, BusDevices, MmioBus, RngDevice, TransferShape, VsockDevice,
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

#[test]
fn a_generation_capture_accepts_only_a_placeholder_volume() {
    let mut bus = bus();
    // A freshly built machine carries the placeholder the Generation declared, which is the
    // device state a capture is allowed to read.
    assert!(require_placeholder_volume(&bus).is_ok());

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
        require_placeholder_volume(&bus),
        Err(SnapshotError::VolumeNotPlaceholder)
    ));
}
