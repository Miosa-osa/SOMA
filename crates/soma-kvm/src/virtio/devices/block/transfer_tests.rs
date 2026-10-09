//! What the block device advertises about request size, and what it does with a request that
//! crosses the limit it enforces.

use super::backend::MemoryBackend;
use super::request::*;
use super::tests::{SERIAL, boot, run};
use super::*;
use crate::virtio::queue::violation::QueueViolationKind;

/// A device that declares the largest request it answers, as machine contract version 2 does.
fn declared(role: BlockRole, sectors: usize) -> BlockDevice {
    let backend = MemoryBackend::zeroed(sectors, role == BlockRole::ImmutableRoot);
    BlockDevice::new(
        role,
        Box::new(backend),
        512,
        SERIAL,
        TransferShape::Declared,
    )
    .expect("device")
}

fn advertised(device: &BlockDevice) -> (u32, u32) {
    let mut raw = [0u8; BLOCK_CONFIG_LEN];
    device.read_config(0, &mut raw).expect("config");
    let size_max = u32::from_le_bytes(raw[8..12].try_into().expect("four bytes"));
    let seg_max = u32::from_le_bytes(raw[12..16].try_into().expect("four bytes"));
    (size_max, seg_max)
}

#[test]
fn a_request_past_a_mebibyte_is_served_on_one_data_segment() {
    // The kernel's own default is 1280 KiB of request. The device used to cap a request at a
    // mebibyte while advertising nothing at all, so the walker dropped a transfer the driver
    // considered legal, the driver waited on a completion that never came, and every later
    // `sync` in that sandbox waited with it.
    let len = 1280 * 1024;
    let sector = 0;
    let (mut rig, mut t) = boot(BlockRole::PrivateOverlay, 4096);
    let payload = vec![0x5a_u8; len];
    let (status, used, _) = run(
        &mut rig,
        &mut t,
        VIRTIO_BLK_T_OUT,
        sector,
        Some((u32::try_from(len).expect("small"), false, &payload)),
    );
    assert_eq!((status, used), (VIRTIO_BLK_S_OK, 1));

    let (status, used, data) = run(
        &mut rig,
        &mut t,
        VIRTIO_BLK_T_IN,
        sector,
        Some((u32::try_from(len).expect("small"), true, &[])),
    );
    assert_eq!(
        (status, used),
        (VIRTIO_BLK_S_OK, u32::try_from(len).expect("small") + 1)
    );
    assert_eq!(rig.read(data, 8), payload[..8].to_vec());
}

#[test]
fn a_request_past_the_protocol_limit_is_answered_and_the_queue_keeps_running() {
    // The refusal belongs to the parser, which answers with a status byte, and not to the
    // walker, which can only drop the chain. A dropped chain is a driver waiting forever, so
    // the walker's cap sits above the parser's limit and this request reaches the parser.
    let len = u32::try_from(MAX_REQUEST_BYTES + 512).expect("small");
    let (mut rig, mut t) = boot(BlockRole::PrivateOverlay, 16384);
    let oversized = vec![0u8; usize::try_from(len).expect("fits")];
    let (status, used, _) = run(
        &mut rig,
        &mut t,
        VIRTIO_BLK_T_OUT,
        0,
        Some((len, false, &oversized)),
    );
    assert_eq!((status, used), (VIRTIO_BLK_S_IOERR, 1));
    assert_eq!(t.device().counters().malformed, 1);
    assert_eq!(
        t.queue(0)
            .expect("queue")
            .violations()
            .count(QueueViolationKind::Chain),
        0,
        "the walker accepted the chain; the parser answered it"
    );

    // And the queue is still usable: the next request runs.
    let (status, used, _) = run(
        &mut rig,
        &mut t,
        VIRTIO_BLK_T_OUT,
        0,
        Some((512, false, &[7u8; 512])),
    );
    assert_eq!((status, used), (VIRTIO_BLK_S_OK, 1));
    assert!(t.is_active());
}

#[test]
fn the_declared_shape_advertises_the_largest_request_it_answers() {
    let device = declared(BlockRole::PrivateOverlay, 8);
    assert_ne!(device.features() & VIRTIO_BLK_F_SIZE_MAX, 0);
    assert_ne!(device.features() & VIRTIO_BLK_F_SEG_MAX, 0);
    let (size_max, seg_max) = advertised(&device);
    assert_eq!(
        u64::from(size_max),
        MAX_REQUEST_BYTES,
        "the limit the driver is told is the limit the parser enforces"
    );
    assert_eq!(seg_max, TRANSFER_SEG_MAX);
    assert!(
        u64::from(size_max) * u64::from(seg_max) <= MAX_REQUEST_BYTES,
        "a driver that respects both fields cannot form a request the parser refuses"
    );
}

#[test]
fn the_undeclared_shape_offers_exactly_the_certified_allowlist() {
    // A snapshot of the version 1 allowlist, every bit named. The digest of the device surface
    // hashes this number, so a version 1 snapshot may only be restored onto a machine whose
    // block devices offer it unchanged. `VIRTIO_F_VERSION_1` is bit 32.
    assert_eq!(
        BlockRole::ImmutableRoot.features(TransferShape::Undeclared),
        (1 << 32) | VIRTIO_BLK_F_RO | VIRTIO_BLK_F_BLK_SIZE
    );
    assert_eq!(
        BlockRole::PrivateOverlay.features(TransferShape::Undeclared),
        (1 << 32) | VIRTIO_BLK_F_BLK_SIZE | VIRTIO_BLK_F_FLUSH
    );
    let device = BlockDevice::new(
        BlockRole::ImmutableRoot,
        Box::new(MemoryBackend::zeroed(8, true)),
        512,
        SERIAL,
        TransferShape::Undeclared,
    )
    .expect("device");
    assert_eq!(advertised(&device), (0, 0));
}
