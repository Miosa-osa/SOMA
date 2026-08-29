//! Randomized hostile requests must never panic, corrupt guest memory outside
//! the chain, or stop the device; plus the real file backend on a temp file.

use super::backend::MemoryBackend;
use super::request::*;
use super::tests::{boot, boot_with, header, run};
use super::*;
use crate::virtio::devices::harness::Seg;
use crate::virtio::devices::service::service_queue;

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

#[test]
fn random_headers_and_shapes_never_panic_or_stop_the_device() {
    let (mut rig, mut t) = boot(BlockRole::PrivateOverlay, 64);
    let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
    for round in 0..300 {
        let mut raw = [0u8; 16];
        for byte in &mut raw {
            *byte = u8::try_from(rng.next() & 0xff).expect("byte");
        }
        if round % 3 == 0 {
            raw[0..4]
                .copy_from_slice(&u32::try_from(rng.next() % 10).expect("small").to_le_bytes());
            raw[8..16].copy_from_slice(&(rng.next() % 80).to_le_bytes());
        }
        let head_len = u32::try_from(8 + rng.next() % 16).expect("small");
        let head = rig.alloc(&raw[..usize::try_from(head_len.min(16)).expect("small")]);
        let data_len = u32::try_from(1 + rng.next() % 2048).expect("small");
        let data = rig.alloc_zeroed(data_len);
        let status = rig.alloc(&[0xaa]);
        let mut segments = vec![Seg::readable(head, head_len.min(16))];
        match rng.next() % 4 {
            0 => segments.push(Seg::readable(data, data_len)),
            1 => segments.push(Seg::writable(data, data_len)),
            _ => {}
        }
        if !rng.next().is_multiple_of(5) {
            segments.push(Seg::writable(status, 1));
        }
        rig.submit(0, &segments);
        rig.notify(&mut t, 0);
        let report = service_queue(&mut t, &rig.mem, 0, 4).expect("service never faults");
        assert_eq!(report.completed + report.rejected, 1);
        assert!(t.is_active());
    }
    let counters = t.device().counters();
    assert_eq!(
        counters.ok + counters.io_error + counters.unsupported + counters.malformed,
        300
    );
}

#[test]
fn a_full_queue_of_requests_is_served_within_budget_and_rescheduled() {
    let (mut rig, mut t) = boot(BlockRole::ImmutableRoot, 64);
    for _ in 0..20 {
        let head = rig.alloc(&header(VIRTIO_BLK_T_IN, 1));
        let data = rig.alloc_zeroed(512);
        let status = rig.alloc(&[0xaa]);
        rig.submit(
            0,
            &[
                Seg::readable(head, 16),
                Seg::writable(data, 512),
                Seg::writable(status, 1),
            ],
        );
    }
    let first = service_queue(&mut t, &rig.mem, 0, 8).expect("service");
    assert_eq!((first.completed, first.exhausted), (8, true));
    let second = service_queue(&mut t, &rig.mem, 0, 64).expect("service");
    assert_eq!((second.completed, second.exhausted), (12, false));
    assert_eq!(rig.used_idx(0), 20);
}

#[test]
fn hostile_chain_shape_after_a_good_request_keeps_the_queue_flowing() {
    let (mut rig, mut t) = boot(BlockRole::ImmutableRoot, 8);
    let (status, _, _) = run(&mut rig, &mut t, VIRTIO_BLK_T_IN, 0, Some((512, true, &[])));
    assert_eq!(status, VIRTIO_BLK_S_OK);
    let head = rig.alloc(&header(VIRTIO_BLK_T_IN, 0));
    rig.submit(0, &[Seg::writable(head, 16), Seg::readable(head, 16)]);
    let report = service_queue(&mut t, &rig.mem, 0, 8).expect("service");
    assert_eq!(report.rejected, 1);
    let (status, _, _) = run(&mut rig, &mut t, VIRTIO_BLK_T_IN, 0, Some((512, true, &[])));
    assert_eq!(status, VIRTIO_BLK_S_OK);
}

#[cfg(unix)]
#[test]
fn file_backend_reads_root_and_writes_flushes_overlay_on_a_temp_file() {
    use super::backend::FileBackend;
    let dir = std::env::temp_dir().join(format!("soma-blk-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let root_path = dir.join("root.img");
    let overlay_path = dir.join("overlay.img");
    let mut image = vec![0u8; 4096 + 100];
    image[1024..1536].fill(0x33);
    std::fs::write(&root_path, &image).expect("root");
    std::fs::write(&overlay_path, vec![0u8; 8192]).expect("overlay");

    let root_file = std::fs::File::open(&root_path).expect("open ro");
    let root = FileBackend::new(root_file, true).expect("backend");
    assert_eq!(
        root.capacity_bytes(),
        4096,
        "trailing partial sector is ignored"
    );
    let device =
        BlockDevice::new(BlockRole::ImmutableRoot, Box::new(root), 512, [0; 20]).expect("dev");
    let (mut rig, mut t) = boot_with(device);
    let (status, used, addr) = run(&mut rig, &mut t, VIRTIO_BLK_T_IN, 2, Some((512, true, &[])));
    assert_eq!((status, used), (VIRTIO_BLK_S_OK, 513));
    assert_eq!(rig.read(addr, 512), vec![0x33u8; 512]);
    let (status, _, _) = run(
        &mut rig,
        &mut t,
        VIRTIO_BLK_T_OUT,
        0,
        Some((512, false, &[1u8; 512])),
    );
    assert_eq!(status, VIRTIO_BLK_S_IOERR);
    assert_eq!(
        std::fs::read(&root_path).expect("root"),
        image,
        "root file unchanged"
    );

    let overlay_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&overlay_path)
        .expect("rw");
    let overlay = FileBackend::new(overlay_file, false).expect("backend");
    let device =
        BlockDevice::new(BlockRole::PrivateOverlay, Box::new(overlay), 512, [0; 20]).expect("dev");
    let (mut rig, mut t) = boot_with(device);
    let payload = [0x77u8; 1024];
    let (status, _, _) = run(
        &mut rig,
        &mut t,
        VIRTIO_BLK_T_OUT,
        4,
        Some((1024, false, &payload)),
    );
    assert_eq!(status, VIRTIO_BLK_S_OK);
    let (status, _, _) = run(&mut rig, &mut t, VIRTIO_BLK_T_FLUSH, 0, None);
    assert_eq!(status, VIRTIO_BLK_S_OK);
    let (status, _, _) = run(
        &mut rig,
        &mut t,
        VIRTIO_BLK_T_IN,
        16,
        Some((512, true, &[])),
    );
    assert_eq!(status, VIRTIO_BLK_S_IOERR, "beyond capacity");
    let written = std::fs::read(&overlay_path).expect("overlay");
    assert_eq!(&written[2048..3072], &payload);
    assert!(written[..2048].iter().all(|b| *b == 0));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = MemoryBackend::default();
}
