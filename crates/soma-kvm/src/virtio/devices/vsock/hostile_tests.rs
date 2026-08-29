//! Every packet rejection class, plus randomized hostile packets that must
//! never panic, fault, or stop the device.

use super::guest_driver::{CID, GUEST_PORT, GuestVsock, connect, hdr};
use super::packet::*;
use crate::virtio::devices::harness::Seg;

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
fn every_header_rejection_class_is_counted_without_a_reply() {
    let mut guest = GuestVsock::boot();
    connect(&mut guest);
    let mut wrong_type = hdr(VSOCK_OP_RW, 0, 0, 0);
    wrong_type.ty = 2;
    let mut invalid_op = hdr(VSOCK_OP_INVALID, 0, 0, 0);
    invalid_op.op = VSOCK_OP_INVALID;
    let mut unknown_op = hdr(8, 0, 0, 0);
    unknown_op.op = 8;
    let mut wrong_src = hdr(VSOCK_OP_RW, 0, 0, 0);
    wrong_src.src_cid = CID + 1;
    let mut wrong_dst = hdr(VSOCK_OP_RW, 0, 0, 0);
    wrong_dst.dst_cid = 3;
    let mut flags_on_rw = hdr(VSOCK_OP_RW, 0, 0, 0);
    flags_on_rw.flags = 1;
    let cases = [
        (wrong_type, &[][..]),
        (invalid_op, &[]),
        (unknown_op, &[]),
        (wrong_src, &[]),
        (wrong_dst, &[]),
        (hdr(VSOCK_OP_RW, 3, 0, 0), &[1, 2]),
        (hdr(VSOCK_OP_RW, 1, 0, 0), &[1, 2]),
        (hdr(VSOCK_OP_CREDIT_UPDATE, 2, 0, 0), &[1, 2]),
        (flags_on_rw, &[]),
        (hdr(VSOCK_OP_SHUTDOWN, 0, 0, 0), &[]),
        (hdr(VSOCK_OP_SHUTDOWN, 0, 4, 0), &[]),
    ];
    let count = u32::try_from(cases.len()).expect("small");
    for (header, payload) in cases {
        guest.send(header, payload);
    }
    let short = guest.rig.alloc(&[0u8; 20]);
    guest.send_raw(&[Seg::readable(short, 20)]);
    let good = guest.rig.alloc(&hdr(VSOCK_OP_RW, 0, 0, 0).to_bytes());
    let sink = guest.rig.alloc_zeroed(8);
    guest.send_raw(&[Seg::readable(good, 44), Seg::writable(sink, 8)]);
    let counters = guest.device().counters();
    assert_eq!(counters.rejected, count + 2);
    assert_eq!(counters.rst_sent, 0, "{counters:?}");
    assert_eq!(
        counters.tx_packets, 1,
        "only the connect REQUEST counted as parsed"
    );
    guest.post_rx(4096);
    assert!(guest.recv().is_empty());
    assert!(guest.device().endpoint().expect("endpoint").is_open());
    assert!(guest.t.is_active());
}

#[test]
fn oversized_payload_is_rejected_before_any_copy() {
    let mut guest = GuestVsock::boot();
    connect(&mut guest);
    let too_big = MAX_PAYLOAD_LEN + 1;
    let header = guest.rig.alloc(&hdr(VSOCK_OP_RW, too_big, 0, 0).to_bytes());
    let data = guest.rig.alloc_zeroed(too_big);
    let report = guest.send_raw(&[Seg::readable(header, 44), Seg::readable(data, too_big)]);
    assert_eq!(
        report.rejected, 1,
        "the chain walker refuses the aggregate length"
    );
    assert_eq!(
        guest.device().endpoint().expect("endpoint").pending_read(),
        0
    );
    let mut lying = hdr(VSOCK_OP_RW, MAX_PAYLOAD_LEN, 0, 0);
    lying.len = MAX_PAYLOAD_LEN;
    guest.send(lying, &[0u8; 10]);
    assert_eq!(guest.device().counters().rejected, 1);
}

#[test]
fn random_packets_never_panic_fault_or_stop_the_device() {
    let mut guest = GuestVsock::boot();
    let mut rng = XorShift(0x1234_5678_9abc_def1);
    for round in 0..400 {
        let mut header = hdr(VSOCK_OP_REQUEST, 0, 0, 0);
        header.op = u16::try_from(rng.next() % 9).expect("small");
        if !rng.next().is_multiple_of(4) {
            header.src_cid = rng.next();
        }
        if !rng.next().is_multiple_of(4) {
            header.dst_cid = rng.next() % 4;
        }
        if !rng.next().is_multiple_of(3) {
            header.dst_port = u32::try_from(rng.next() % 6000).expect("small");
        }
        header.src_port = GUEST_PORT + u32::try_from(rng.next() % 3).expect("small");
        header.flags = u32::try_from(rng.next() % 5).expect("small");
        header.buf_alloc = u32::try_from(rng.next() & 0xffff).expect("small");
        header.fwd_cnt = u32::try_from(rng.next() & 0xff).expect("small");
        let payload_len = usize::try_from(rng.next() % 300).expect("small");
        header.len = if rng.next().is_multiple_of(2) {
            u32::try_from(payload_len).expect("small")
        } else {
            u32::try_from(rng.next() & 0xffff).expect("small")
        };
        let payload = vec![u8::try_from(round % 251).expect("byte"); payload_len];
        guest.send(header, &payload);
        if round % 7 == 0 {
            guest.post_rx(u32::try_from(1 + rng.next() % 200).expect("small"));
            guest.post_rx(4096);
            let _ = guest.recv();
        }
        if round % 50 == 0
            && let Some(endpoint) = guest.device().endpoint()
        {
            let mut buf = [0u8; 64];
            let _ = endpoint.read(&mut buf);
            let _ = endpoint.write(&buf);
        }
        assert!(guest.t.is_active());
    }
    let counters = guest.device().counters();
    assert!(counters.rejected + counters.tx_packets >= 1);
}
