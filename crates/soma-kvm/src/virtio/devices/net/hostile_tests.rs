//! Randomized hostile transmit chains and receive shapes never panic or stop
//! the device, and the TAP backend moves frames over a real pipe.

use super::backend::LoopbackBackend;
use super::frame::VIRTIO_NET_HDR_LEN;
use super::rx::deliver_rx;
use super::tests::{MAC, boot, frame};
use super::*;
use crate::virtio::devices::harness::{GuestRig, Seg};
use crate::virtio::devices::service::service_queue;
use crate::virtio::transport::MmioTransport;

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

    fn below(&mut self, bound: u64) -> usize {
        usize::try_from(self.next() % bound).expect("small")
    }
}

#[test]
fn random_transmit_chains_never_panic_or_stop_the_device() {
    let (mut rig, mut t, _) = boot(false, true);
    let mut rng = XorShift(0x2545_f491_4f6c_dd1d);
    for _ in 0..300 {
        let mut header = [0u8; VIRTIO_NET_HDR_LEN];
        if rng.below(4) == 0 {
            header[rng.below(12)] = 1;
        }
        let hdr_len = u32::try_from(1 + rng.below(16)).expect("small");
        let hdr = rig.alloc(&header[..usize::try_from(hdr_len.min(12)).expect("small")]);
        let mut segments = vec![Seg::readable(hdr, hdr_len.min(12))];
        for _ in 0..rng.below(3) {
            let len = u32::try_from(1 + rng.below(1600)).expect("small");
            let data = rig.alloc_zeroed(len);
            let writable = rng.below(8) == 0;
            segments.push(Seg {
                addr: data,
                len,
                writable,
            });
        }
        rig.submit(NET_TX_QUEUE, &segments);
        let report = service_queue(&mut t, &rig.mem, NET_TX_QUEUE, 4).expect("never faults");
        assert_eq!(report.completed + report.rejected, 1);
        assert!(t.is_active());
    }
    let counters = t.device().counters();
    assert!(counters.tx_ok + counters.tx_dropped <= 300);
}

#[test]
fn random_receive_shapes_never_panic_and_never_overrun_buffers() {
    let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
    for round in 0..40 {
        let backend = LoopbackBackend::default();
        let host = backend.handle();
        for _ in 0..8 {
            let len = 1 + rng.below(1600);
            host.push_inbound(&frame(len, 0xee));
        }
        let mut device = NetDevice::new(Box::new(backend), MAC);
        device.set_link(true);
        let mut rig = GuestRig::new(&[32, 32]);
        let mut t = MmioTransport::new(device).expect("transport");
        rig.init(&mut t, NET_FEATURES);
        let mut posted = Vec::new();
        for _ in 0..=(round % 8) {
            let len = u32::try_from(1 + rng.below(2100)).expect("small");
            let addr = rig.alloc_zeroed(len + 32);
            rig.submit(NET_RX_QUEUE, &[Seg::writable(addr, len)]);
            posted.push((addr, len));
        }
        let report = deliver_rx(&mut t, &rig.mem, 16).expect("never faults");
        assert!(report.completed <= 8);
        for (index, (addr, len)) in posted.iter().enumerate() {
            let used = if u16::try_from(index).expect("small") < rig.used_idx(NET_RX_QUEUE) {
                rig.used_elem(NET_RX_QUEUE, u16::try_from(index).expect("small"))
                    .1
            } else {
                0
            };
            assert!(used <= *len, "used {used} never exceeds capacity {len}");
            let guard = rig.read(addr + u64::from(*len), 16);
            assert_eq!(guard, [0u8; 16], "no write past the buffer");
        }
        assert!(t.is_active());
    }
}

#[cfg(unix)]
#[test]
fn tap_backend_moves_frames_over_a_pipe() {
    use super::backend::{NetBackend, TapBackend};
    use std::fs::File;
    let (reader, writer) = std::io::pipe().expect("pipe");
    let mut tx = TapBackend::new(File::from(std::os::fd::OwnedFd::from(writer)));
    let mut rx = TapBackend::new(File::from(std::os::fd::OwnedFd::from(reader)));
    let frame = frame(120, 0x99);
    tx.transmit(&frame).expect("transmit");
    let mut buf = [0u8; 2048];
    assert_eq!(rx.receive(&mut buf), Ok(Some(120)));
    assert_eq!(&buf[..120], &frame[..]);
    drop(tx);
    assert_eq!(
        rx.receive(&mut buf),
        Ok(None),
        "closed writer reads as idle"
    );
}
