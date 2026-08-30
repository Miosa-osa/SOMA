//! End-to-end block tests through the transport with a model driver.

use super::backend::MemoryBackend;
use super::request::*;
use super::*;
use crate::virtio::devices::harness::{GuestRig, Seg};
use crate::virtio::devices::service::service_queue;
use crate::virtio::queue::violation::QueueViolationKind;
use crate::virtio::transport::MmioTransport;
use crate::virtio::transport::registers::{AccessWidth, REG_INTERRUPT_STATUS};

pub(super) const SERIAL: [u8; BLOCK_SERIAL_LEN] = *b"soma-test-serial-001";
type Case<'a> = (u32, u64, Option<(u32, bool, &'a [u8])>, u8);

pub(super) fn device(role: BlockRole, sectors: usize) -> BlockDevice {
    let backend = MemoryBackend::zeroed(sectors, role == BlockRole::ImmutableRoot);
    BlockDevice::new(role, Box::new(backend), 512, SERIAL).expect("device")
}

pub(super) fn boot(role: BlockRole, sectors: usize) -> (GuestRig, MmioTransport<BlockDevice>) {
    boot_with(device(role, sectors))
}

/// A fresh rig per transport: the rig's available index must start with the queue.
pub(super) fn boot_with(device: BlockDevice) -> (GuestRig, MmioTransport<BlockDevice>) {
    let rig = GuestRig::new(&[64]);
    let features = device.role().features();
    let mut t = MmioTransport::new(device).expect("transport");
    rig.init(&mut t, features);
    (rig, t)
}

pub(super) fn header(ty: u32, sector: u64) -> [u8; 16] {
    let mut raw = [0u8; 16];
    raw[0..4].copy_from_slice(&ty.to_le_bytes());
    raw[8..16].copy_from_slice(&sector.to_le_bytes());
    raw
}

/// Submits header + optional data + status, services the queue, and returns
/// `(status byte, used length, data address)`.
pub(super) fn run(
    rig: &mut GuestRig,
    t: &mut MmioTransport<BlockDevice>,
    ty: u32,
    sector: u64,
    data: Option<(u32, bool, &[u8])>,
) -> (u8, u32, u64) {
    let head = rig.alloc(&header(ty, sector));
    let status = rig.alloc(&[0xaa]);
    let mut segments = vec![Seg::readable(head, 16)];
    let mut data_addr = 0;
    if let Some((len, writable, bytes)) = data {
        data_addr = rig.alloc_zeroed(len);
        if !bytes.is_empty() {
            rig.mem
                .write_bytes(crate::virtio::guest_memory::GuestAddress(data_addr), bytes)
                .expect("data");
        }
        segments.push(Seg {
            addr: data_addr,
            len,
            writable,
        });
    }
    segments.push(Seg::writable(status, 1));
    let before = rig.used_idx(0);
    rig.submit(0, &segments);
    rig.notify(t, 0);
    let report = service_queue(t, &rig.mem, 0, 8).expect("service");
    assert_eq!(report.completed, 1);
    assert_eq!(rig.used_idx(0), before.wrapping_add(1));
    let (_, used) = rig.used_elem(0, before);
    (rig.read(status, 1)[0], used, data_addr)
}

#[test]
fn root_read_returns_backend_bytes_and_used_length() {
    let mut pattern = [0u8; 1024];
    for (index, byte) in pattern.iter_mut().enumerate() {
        *byte = u8::try_from(index % 251).expect("small");
    }
    let mut backend = MemoryBackend::zeroed(8, true);
    backend.bytes[1024..2048].copy_from_slice(&pattern);
    let (mut rig, mut t) = boot_with(
        BlockDevice::new(BlockRole::ImmutableRoot, Box::new(backend), 512, SERIAL).expect("dev"),
    );
    let (status, used, addr) = run(
        &mut rig,
        &mut t,
        VIRTIO_BLK_T_IN,
        2,
        Some((1024, true, &[])),
    );
    assert_eq!(status, VIRTIO_BLK_S_OK);
    assert_eq!(used, 1025);
    assert_eq!(rig.read(addr, 1024), pattern);
    assert_eq!(t.read(REG_INTERRUPT_STATUS, AccessWidth::U32), Ok(1));
    assert_eq!(t.device().counters().ok, 1);
}

#[test]
fn overlay_write_and_flush_reach_backend() {
    let (mut rig, mut t) = boot(BlockRole::PrivateOverlay, 8);
    let payload = [0x5au8; 512];
    let (status, used, _) = run(
        &mut rig,
        &mut t,
        VIRTIO_BLK_T_OUT,
        3,
        Some((512, false, &payload)),
    );
    assert_eq!((status, used), (VIRTIO_BLK_S_OK, 1));
    let (status, used, addr) = run(&mut rig, &mut t, VIRTIO_BLK_T_IN, 3, Some((512, true, &[])));
    assert_eq!((status, used), (VIRTIO_BLK_S_OK, 513));
    assert_eq!(rig.read(addr, 512), payload);
    let (status, used, _) = run(&mut rig, &mut t, VIRTIO_BLK_T_FLUSH, 0, None);
    assert_eq!((status, used), (VIRTIO_BLK_S_OK, 1));
    assert_eq!(t.device().counters().ok, 3);
}

#[test]
fn root_write_is_ioerr_and_flush_is_unsupported() {
    let (mut rig, mut t) = boot(BlockRole::ImmutableRoot, 8);
    let payload = [1u8; 512];
    let (status, used, _) = run(
        &mut rig,
        &mut t,
        VIRTIO_BLK_T_OUT,
        0,
        Some((512, false, &payload)),
    );
    assert_eq!((status, used), (VIRTIO_BLK_S_IOERR, 1));
    let (status, _, addr) = run(&mut rig, &mut t, VIRTIO_BLK_T_IN, 0, Some((512, true, &[])));
    assert_eq!(status, VIRTIO_BLK_S_OK);
    assert_eq!(rig.read(addr, 512), [0u8; 512], "backend untouched");
    let (status, _, _) = run(&mut rig, &mut t, VIRTIO_BLK_T_FLUSH, 0, None);
    assert_eq!(status, VIRTIO_BLK_S_UNSUPP);
    assert_eq!(t.device().counters().unsupported, 1);
    assert_eq!(t.device().counters().malformed, 1);
}

#[test]
fn every_rejection_class_reports_a_status_byte() {
    let (mut rig, mut t) = boot(BlockRole::PrivateOverlay, 8);
    let cases: [Case<'_>; 9] = [
        (3, 0, None, VIRTIO_BLK_S_UNSUPP),
        (
            VIRTIO_BLK_T_IN,
            0,
            Some((512, false, &[0u8; 512])),
            VIRTIO_BLK_S_IOERR,
        ),
        (
            VIRTIO_BLK_T_OUT,
            0,
            Some((512, true, &[])),
            VIRTIO_BLK_S_IOERR,
        ),
        (
            VIRTIO_BLK_T_IN,
            u64::MAX / 512 + 1,
            Some((512, true, &[])),
            VIRTIO_BLK_S_IOERR,
        ),
        (
            VIRTIO_BLK_T_IN,
            u64::MAX / 512,
            Some((512, true, &[])),
            VIRTIO_BLK_S_IOERR,
        ),
        (
            VIRTIO_BLK_T_IN,
            0,
            Some((100, true, &[])),
            VIRTIO_BLK_S_IOERR,
        ),
        (
            VIRTIO_BLK_T_IN,
            8,
            Some((512, true, &[])),
            VIRTIO_BLK_S_IOERR,
        ),
        (
            VIRTIO_BLK_T_FLUSH,
            0,
            Some((512, true, &[])),
            VIRTIO_BLK_S_IOERR,
        ),
        (
            VIRTIO_BLK_T_GET_ID,
            0,
            Some((21, true, &[])),
            VIRTIO_BLK_S_IOERR,
        ),
    ];
    for (ty, sector, data, expected) in cases {
        let (status, used, _) = run(&mut rig, &mut t, ty, sector, data);
        assert_eq!((status, used), (expected, 1), "type {ty} sector {sector}");
    }
    assert_eq!(t.device().counters().malformed, 8);
    assert_eq!(t.device().counters().unsupported, 1);
}

#[test]
fn short_header_and_missing_status_are_handled_without_backend_io() {
    let (mut rig, mut t) = boot(BlockRole::PrivateOverlay, 8);
    let short = rig.alloc(&[0u8; 8]);
    let status = rig.alloc(&[0xaa]);
    rig.submit(0, &[Seg::readable(short, 8), Seg::writable(status, 1)]);
    let head = rig.alloc(&header(VIRTIO_BLK_T_FLUSH, 0));
    rig.submit(0, &[Seg::readable(head, 16)]);
    let report = service_queue(&mut t, &rig.mem, 0, 8).expect("service");
    assert_eq!(report.completed, 2);
    assert_eq!(rig.read(status, 1)[0], VIRTIO_BLK_S_IOERR);
    assert_eq!(rig.used_elem(0, 1).1, 0);
    assert_eq!(t.device().counters().malformed, 2);
}

#[test]
fn oversized_chain_is_rejected_by_the_walker_and_counted() {
    let (mut rig, mut t) = boot(BlockRole::PrivateOverlay, 4096);
    let head = rig.alloc(&header(VIRTIO_BLK_T_IN, 0));
    let data = rig.alloc_zeroed(u32::try_from(MAX_REQUEST_BYTES + 512).expect("small"));
    let status = rig.alloc(&[0xaa]);
    rig.submit(
        0,
        &[
            Seg::readable(head, 16),
            Seg::writable(data, u32::try_from(MAX_REQUEST_BYTES + 512).expect("small")),
            Seg::writable(status, 1),
        ],
    );
    let report = service_queue(&mut t, &rig.mem, 0, 8).expect("service");
    assert_eq!((report.completed, report.rejected), (0, 1));
    assert_eq!(
        t.queue(0)
            .expect("queue")
            .violations()
            .count(QueueViolationKind::Chain),
        1
    );
    assert_eq!(rig.read(status, 1)[0], 0xaa);
}

#[test]
fn short_host_io_and_backend_failure_are_io_errors() {
    let mut backend = MemoryBackend::zeroed(8, false);
    backend.short_by = 1;
    let device =
        BlockDevice::new(BlockRole::PrivateOverlay, Box::new(backend), 512, SERIAL).expect("dev");
    let (mut rig, mut t) = boot_with(device);
    let (status, used, _) = run(&mut rig, &mut t, VIRTIO_BLK_T_IN, 0, Some((512, true, &[])));
    assert_eq!((status, used), (VIRTIO_BLK_S_IOERR, 1));
    let (status, _, _) = run(
        &mut rig,
        &mut t,
        VIRTIO_BLK_T_OUT,
        0,
        Some((512, false, &[7u8; 512])),
    );
    assert_eq!(status, VIRTIO_BLK_S_IOERR);

    let mut backend = MemoryBackend::zeroed(8, false);
    backend.fail = Some(std::io::ErrorKind::Other);
    let device =
        BlockDevice::new(BlockRole::PrivateOverlay, Box::new(backend), 512, SERIAL).expect("dev");
    let (mut rig, mut t) = boot_with(device);
    let (status, _, _) = run(&mut rig, &mut t, VIRTIO_BLK_T_FLUSH, 0, None);
    assert_eq!(status, VIRTIO_BLK_S_IOERR);
    assert_eq!(t.device().counters().io_error, 1);
    assert!(t.is_active(), "per-request failures never stop the device");
}

#[test]
fn get_id_fills_the_serial_and_interrupt_suppression_is_honored() {
    let (mut rig, mut t) = boot(BlockRole::ImmutableRoot, 8);
    rig.set_no_interrupt(0, true);
    let head = rig.alloc(&header(VIRTIO_BLK_T_GET_ID, 0));
    let id = rig.alloc_zeroed(20);
    let status = rig.alloc(&[0xaa]);
    rig.submit(
        0,
        &[
            Seg::readable(head, 16),
            Seg::writable(id, 20),
            Seg::writable(status, 1),
        ],
    );
    let report = service_queue(&mut t, &rig.mem, 0, 8).expect("service");
    assert!(!report.interrupt);
    assert_eq!(rig.read(id, 20), SERIAL);
    assert_eq!(rig.used_elem(0, 0).1, 21);
}

mod detached {
    use super::super::backend::{Detached, MemoryBackend};
    use super::super::{BlockConfigError, BlockDevice, BlockRole};
    use super::SERIAL;
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
        assert_eq!(backend.write_at(0, &[0; 512]), Err(BackendError::OutOfRange));
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
}
