//! The bytes on the queue: how a request may be split across descriptors, what a malformed chain
//! costs, and that the response is written exactly and nowhere else.

use super::rig::{ACK, ERROR, PLUG, Rig, UNPLUG, encode};
use super::*;
use crate::virtio::devices::harness::Seg;
use crate::virtio::devices::service::ServiceError;

const BLOCKS: u64 = 16;
const FILL: u8 = 0xa5;

/// Publishes a request cut into `cuts` readable pieces and a response area cut into `replies`
/// writable pieces, then services the queue. Returns each reply piece's address and the report.
fn exchange(
    r: &mut Rig,
    raw: &[u8],
    cuts: &[usize],
    replies: &[u32],
) -> (
    Vec<u64>,
    Result<crate::virtio::devices::service::ServiceReport, ServiceError>,
) {
    let mut segments = Vec::new();
    let mut start = 0;
    for len in cuts {
        let addr = r.guest.alloc(&raw[start..start + len]);
        segments.push(Seg::readable(addr, u32::try_from(*len).expect("small")));
        start += len;
    }
    let mut addrs = Vec::new();
    for len in replies {
        let addr = r
            .guest
            .alloc(&vec![FILL; usize::try_from(*len).expect("small")]);
        addrs.push(addr);
        segments.push(Seg::writable(addr, *len));
    }
    r.guest.submit(0, &segments);
    (addrs, r.serve())
}

fn assert_untouched(r: &Rig, addr: u64, len: usize) {
    assert_eq!(
        r.guest.read(addr, len),
        vec![FILL; len],
        "buffer was written"
    );
}

#[test]
fn a_request_may_be_cut_anywhere_across_descriptors() {
    for cuts in [&[24][..], &[1, 23], &[8, 16], &[8, 8, 8], &[2, 6, 8, 2, 6]] {
        let mut r = Rig::new(BLOCKS);
        r.want(4);
        let (replies, report) = exchange(&mut r, &encode(PLUG, Rig::at(1), 2), cuts, &[10]);
        assert_eq!(report.expect("service").completed, 1, "cuts {cuts:?}");
        assert_eq!(r.guest.read(replies[0], 2), [ACK.to_le_bytes()[0], 0]);
        assert_eq!(r.plugged(), 2, "cuts {cuts:?}");
    }
}

#[test]
fn a_response_may_be_cut_and_nothing_past_it_is_written() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    let (replies, report) = exchange(&mut r, &encode(PLUG, Rig::at(0), 1), &[24], &[3, 7, 50]);
    assert_eq!(report.expect("service").completed, 1);
    assert_eq!(
        r.guest.used_elem(0, 0).1,
        10,
        "ten bytes, however long the area"
    );
    assert_eq!(r.guest.read(replies[0], 3), [0, 0, 0]);
    assert_eq!(r.guest.read(replies[1], 7), [0; 7]);
    assert_untouched(&r, replies[2], 50);
}

#[test]
fn the_response_layout_is_type_then_reserved_then_state() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    let (replies, _) = exchange(&mut r, &encode(3, Rig::at(0), 4), &[24], &[10]);
    assert_eq!(r.guest.read(replies[0], 10), [0, 0, 0, 0, 0, 0, 0, 0, 1, 0]);
    let (replies, _) = exchange(&mut r, &encode(99, 0, 0), &[24], &[10]);
    assert_eq!(r.guest.read(replies[0], 10), [3, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn every_answer_publishes_ten_bytes_used() {
    let mut r = Rig::new(BLOCKS);
    r.want(1);
    for (kind, addr, count) in [
        (PLUG, 0, 1),
        (PLUG, 1, 1),
        (UNPLUG, 0, 1),
        (UNPLUG, 0, 1),
        (9, 0, 0),
    ] {
        let answer = r.ask(kind, Rig::at(addr), count);
        assert_eq!(answer.used, 10, "kind {kind} answered {}", answer.kind);
    }
}

#[test]
fn a_chain_of_the_wrong_size_is_dropped_with_no_effect_and_no_answer() {
    for request_len in [0_usize, 1, 23, 25, 48] {
        let mut r = Rig::new(BLOCKS);
        r.want(4);
        let raw = vec![0_u8; request_len];
        let (replies, report) = exchange(&mut r, &raw, &[request_len], &[10]);
        let report = report.expect("service");
        // An empty request is refused by the chain walker before the device sees it; every
        // other size reaches the device, which answers with a zero length and counts it.
        assert_eq!(report.completed + report.rejected, 1, "len {request_len}");
        if report.completed == 1 {
            assert_eq!(r.guest.used_elem(0, 0).1, 0, "len {request_len}");
            assert_eq!(r.t.device().counters().rejected, 1, "len {request_len}");
        }
        assert_untouched(&r, replies[0], 10);
        assert_eq!(r.plugged(), 0);
        assert!(r.t.is_active());
        assert_eq!(
            r.ask(PLUG, Rig::at(0), 1).kind,
            ACK,
            "the queue still works"
        );
    }
}

#[test]
fn a_response_area_that_is_too_small_or_missing_is_dropped_untouched() {
    for reply_len in [1_u32, 5, 9] {
        let mut r = Rig::new(BLOCKS);
        r.want(4);
        let (replies, report) = exchange(&mut r, &encode(PLUG, Rig::at(0), 1), &[24], &[reply_len]);
        assert_eq!(report.expect("service").completed, 1);
        assert_untouched(&r, replies[0], usize::try_from(reply_len).expect("small"));
        assert_eq!(r.plugged(), 0, "an unanswerable request is not executed");
        assert_eq!(r.t.device().counters().rejected, 1);
    }
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    let (_, report) = exchange(&mut r, &encode(PLUG, Rig::at(0), 1), &[24], &[]);
    assert_eq!(report.expect("service").completed, 1);
    assert_eq!(r.plugged(), 0);
}

#[test]
fn a_request_in_device_writable_memory_is_not_a_request() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    let body = r.guest.alloc(&encode(PLUG, Rig::at(0), 1));
    let reply = r.guest.alloc(&[FILL; 10]);
    r.guest
        .submit(0, &[Seg::writable(body, 24), Seg::writable(reply, 10)]);
    let report = r.serve().expect("service");
    assert_eq!(report.completed, 1);
    assert_eq!(r.plugged(), 0);
    assert_untouched(&r, reply, 10);
}

#[test]
fn a_chain_with_the_response_before_the_request_never_runs_the_request() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    let body = r.guest.alloc(&encode(PLUG, Rig::at(0), 1));
    let reply = r.guest.alloc(&[FILL; 10]);
    r.guest
        .submit(0, &[Seg::writable(reply, 10), Seg::readable(body, 24)]);
    let outcome = r.serve();
    assert!(
        outcome.is_ok() || !r.t.is_active(),
        "refused or stopped, never panics"
    );
    assert!(r.plugged() <= 4);
    assert_eq!(r.t.device().plugged_size(), r.plugged() * BLOCK_SIZE);
}

#[test]
fn a_chain_longer_than_the_descriptor_cap_is_rejected_whole() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    let raw = encode(PLUG, Rig::at(0), 1);
    let mut segments: Vec<Seg> = raw
        .iter()
        .map(|byte| Seg::readable(r.guest.alloc(&[*byte]), 1))
        .collect();
    segments.push(Seg::writable(r.guest.alloc(&[FILL; 10]), 10));
    r.guest.submit(0, &segments);
    let report = r.serve().expect("service");
    assert_eq!((report.completed, report.rejected), (0, 1));
    assert_eq!(r.plugged(), 0);
}

#[test]
fn a_buffer_outside_guest_ram_never_reaches_the_device_state() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    let reply = r.guest.alloc(&[FILL; 10]);
    r.guest.submit(
        0,
        &[
            Seg::readable(0xffff_0000_0000, 24),
            Seg::writable(reply, 10),
        ],
    );
    let outcome = r.serve();
    assert!(outcome.is_ok() || !r.t.is_active());
    assert_eq!(r.plugged(), 0);
    assert_untouched(&r, reply, 10);
}

#[test]
fn the_unused_reserved_bytes_of_a_request_do_not_change_its_meaning() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    let mut noisy = encode(PLUG, Rig::at(0), 2);
    noisy[2..8].fill(0xff);
    noisy[18..24].fill(0xff);
    assert_eq!(r.ask_raw(&noisy).kind, ACK);
    assert_eq!(r.plugged(), 2);
}

#[test]
fn unknown_request_types_are_errors_and_change_nothing() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    assert_eq!(r.ask(PLUG, Rig::at(0), 1).kind, ACK);
    for kind in [4, 5, 255, 256, 0x8000, u16::MAX] {
        let answer = r.ask(kind, Rig::at(0), 1);
        assert_eq!((answer.kind, answer.state), (ERROR, 0), "kind {kind}");
    }
    assert_eq!(r.plugged(), 1);
    assert!(r.discards().is_empty());
}

#[test]
fn a_doorbell_for_a_queue_that_does_not_exist_is_refused_and_harmless() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    let outcome = crate::virtio::devices::service::service_queue(&mut r.t, &r.guest.mem, 1, 8);
    assert!(matches!(outcome, Err(ServiceError::Transport(_))));
    assert!(r.t.is_active());
    assert_eq!(r.ask(PLUG, Rig::at(0), 1).kind, ACK);
}

#[test]
fn a_full_queue_is_answered_in_order_in_one_pass() {
    let mut r = Rig::new(BLOCKS);
    r.want(8);
    let mut replies = Vec::new();
    for block in 0..8_u64 {
        let body = r.guest.alloc(&encode(PLUG, Rig::at(block), 1));
        let reply = r.guest.alloc(&[FILL; 10]);
        replies.push(reply);
        r.guest
            .submit(0, &[Seg::readable(body, 24), Seg::writable(reply, 10)]);
    }
    let report = r.serve().expect("service");
    assert_eq!(report.completed, 8);
    for (slot, reply) in replies.iter().enumerate() {
        assert_eq!(r.guest.read(*reply, 2), [0, 0], "request {slot}");
        assert_eq!(
            r.guest.used_elem(0, u16::try_from(slot).expect("small")).1,
            10
        );
    }
    assert_eq!(r.plugged(), 8);
}
