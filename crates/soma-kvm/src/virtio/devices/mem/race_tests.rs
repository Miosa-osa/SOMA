//! Things that happen between a request and its service: the host changing its mind, a reset in
//! the middle, a reset twice, and a configuration change the driver has not read yet.
//!
//! The device decides every request when it services it, against the state it has then, not the
//! state at the moment the guest published it. These tests pin that down.

use std::sync::atomic::Ordering;

use super::rig::{ACK, ERROR, NACK, PLUG, Rig, UNPLUG, encode};
use super::*;
use crate::virtio::device::ActivateError;
use crate::virtio::devices::harness::{GuestRig, Seg};
use crate::virtio::devices::service::service_queue;
use crate::virtio::transport::registers::{AccessWidth, REG_CONFIG_GENERATION, REG_STATUS};

const BLOCKS: u64 = 16;

/// Publishes a request without ringing the doorbell; returns where its answer will land.
fn publish(r: &mut Rig, kind: u16, block: u64, count: u16) -> u64 {
    let body = r.guest.alloc(&encode(kind, Rig::at(block), count));
    let reply = r.guest.alloc(&[0xa5; 10]);
    r.guest
        .submit(0, &[Seg::readable(body, 24), Seg::writable(reply, 10)]);
    reply
}

fn answer(r: &Rig, reply: u64) -> u16 {
    let raw = r.guest.read(reply, 2);
    u16::from_le_bytes([raw[0], raw[1]])
}

fn reboot(r: &mut Rig) {
    r.guest = GuestRig::new(&[16]);
    r.guest.init(&mut r.t, MEM_FEATURES);
}

#[test]
fn a_plug_is_judged_against_the_size_asked_when_it_is_serviced() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    let reply = publish(&mut r, PLUG, 0, 4);
    r.want(2);
    r.serve().expect("service");
    assert_eq!(
        answer(&r, reply),
        NACK,
        "the host lowered its request in between"
    );
    assert_eq!(r.plugged(), 0);
}

#[test]
fn a_plug_published_too_early_is_granted_once_the_host_has_asked() {
    let mut r = Rig::new(BLOCKS);
    let reply = publish(&mut r, PLUG, 0, 3);
    r.want(3);
    r.serve().expect("service");
    assert_eq!(answer(&r, reply), ACK);
    assert_eq!(r.plugged(), 3);
}

#[test]
fn an_unplug_does_not_depend_on_what_the_host_wants() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    assert_eq!(r.ask(PLUG, Rig::at(0), 4).kind, ACK);
    let reply = publish(&mut r, UNPLUG, 0, 4);
    r.want(16);
    r.serve().expect("service");
    assert_eq!(
        answer(&r, reply),
        ACK,
        "giving memory back is always allowed"
    );
    assert_eq!(r.plugged(), 0);
}

#[test]
fn two_requests_for_the_same_block_are_decided_in_the_order_they_were_published() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    let first = publish(&mut r, PLUG, 5, 1);
    let second = publish(&mut r, PLUG, 5, 1);
    let third = publish(&mut r, UNPLUG, 5, 1);
    let fourth = publish(&mut r, UNPLUG, 5, 1);
    assert_eq!(r.serve().expect("service").completed, 4);
    let kinds: Vec<u16> = [first, second, third, fourth]
        .iter()
        .map(|a| answer(&r, *a))
        .collect();
    assert_eq!(kinds, [ACK, ERROR, ACK, ERROR]);
    assert_eq!(r.plugged(), 0);
}

#[test]
fn a_refused_plug_does_not_spoil_the_one_queued_behind_it() {
    let mut r = Rig::new(BLOCKS);
    r.want(2);
    let big = publish(&mut r, PLUG, 0, 4);
    let small = publish(&mut r, PLUG, 8, 2);
    r.serve().expect("service");
    assert_eq!((answer(&r, big), answer(&r, small)), (NACK, ACK));
    assert_eq!(r.plugged(), 2);
}

#[test]
fn the_host_changing_its_mind_between_service_passes_applies_to_the_next_request_only() {
    let mut r = Rig::new(BLOCKS);
    r.want(2);
    let replies: Vec<u64> = (0..4)
        .map(|block| publish(&mut r, PLUG, block, 1))
        .collect();
    r.guest.notify(&mut r.t, 0);
    for (pass, target) in [(0, 2), (1, 2), (2, 4), (3, 4)] {
        r.want(target);
        let report = service_queue(&mut r.t, &r.guest.mem, 0, 1).expect("service");
        assert_eq!(report.completed, 1, "pass {pass}");
    }
    let kinds: Vec<u16> = replies.iter().map(|a| answer(&r, *a)).collect();
    assert_eq!(kinds, [ACK, ACK, ACK, ACK]);
    assert_eq!(r.plugged(), 4);
}

#[test]
fn a_reset_with_requests_still_in_the_ring_executes_none_of_them() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    assert_eq!(r.ask(PLUG, Rig::at(0), 2).kind, ACK);
    let _ = publish(&mut r, PLUG, 2, 2);
    let _ = publish(&mut r, UNPLUG, 0, 2);
    reboot(&mut r);
    assert_eq!(r.serve().expect("service").completed, 0);
    assert_eq!(r.plugged(), 0);
    assert_eq!(
        r.discards(),
        vec![(0, BLOCKS * BLOCK_SIZE)],
        "one discard, from the reset"
    );
}

#[test]
fn resetting_twice_is_the_same_as_resetting_once() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    assert_eq!(r.ask(PLUG, Rig::at(0), 4).kind, ACK);
    reboot(&mut r);
    reboot(&mut r);
    assert_eq!(
        r.discards().len(),
        1,
        "the second reset had nothing to return"
    );
    assert_eq!(r.plugged(), 0);
    assert_eq!(r.ask(PLUG, Rig::at(0), 4).kind, ACK);
}

#[test]
fn a_reset_the_host_cannot_honour_keeps_the_blocks_tracked_until_it_can() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    assert_eq!(r.ask(PLUG, Rig::at(0), 4).kind, ACK);
    r.fail.store(true, Ordering::Relaxed);
    // The guest writes status 0, the way a reboot starts.
    r.t.write(REG_STATUS, AccessWidth::U32, 0, &r.guest.mem)
        .expect("reset");
    assert_eq!(
        r.plugged(),
        4,
        "the host still holds the memory, so it stays counted"
    );
    assert!(!r.t.device().is_quiescent(), "and a capture stays refused");
    assert_eq!(r.t.device().counters().discard_failed, 1);
    // A driver may not start over memory the device does not account for.
    assert_eq!(
        r.t.device_mut().activate(MEM_FEATURES),
        Err(ActivateError::BackendUnavailable)
    );
    // The host recovers: the next driver starts from nothing and can plug the same blocks.
    r.fail.store(false, Ordering::Relaxed);
    reboot(&mut r);
    assert!(r.t.device().is_quiescent());
    assert_eq!(r.discards().last(), Some(&(0, BLOCKS * BLOCK_SIZE)));
    assert_eq!(r.ask(PLUG, Rig::at(0), 4).kind, ACK);
}

#[test]
fn several_changes_before_the_driver_looks_leave_it_the_last_one() {
    let mut r = Rig::new(BLOCKS);
    let before =
        r.t.read(REG_CONFIG_GENERATION, AccessWidth::U32)
            .expect("gen");
    for target in [2, 5, 1] {
        r.want(target);
    }
    let after =
        r.t.read(REG_CONFIG_GENERATION, AccessWidth::U32)
            .expect("gen");
    assert_eq!(
        after,
        (before + 3) & 0xffff_ffff,
        "every change is visible to a torn read"
    );
    assert_eq!(r.t.device().requested_size(), BLOCK_SIZE);
    assert_eq!(
        r.ask(PLUG, Rig::at(0), 2).kind,
        NACK,
        "only the last size counts"
    );
    assert_eq!(r.ask(PLUG, Rig::at(0), 1).kind, ACK);
}

#[test]
fn a_doorbell_with_nothing_published_does_nothing() {
    let mut r = Rig::new(BLOCKS);
    r.want(2);
    for _ in 0..3 {
        let report = r.serve().expect("service");
        assert_eq!((report.completed, report.rejected), (0, 0));
    }
    assert_eq!(r.plugged(), 0);
}
