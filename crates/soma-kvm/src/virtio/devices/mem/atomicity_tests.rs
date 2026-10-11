//! A request is all or nothing, the guest never holds more than the host asked for, and a host
//! that cannot take memory back never lets the device claim it did.

use std::sync::atomic::Ordering;

use super::rig::{
    ACK, ERROR, NACK, PLUG, Rig, ST_MIXED, ST_PLUGGED, ST_UNPLUGGED, STATE, UNPLUG, UNPLUG_ALL,
};
use super::*;

const BLOCKS: u64 = 16;

fn state(r: &mut Rig, first: u64, count: u16) -> u16 {
    let answer = r.ask(STATE, Rig::at(first), count);
    assert_eq!(answer.kind, ACK);
    answer.state
}

#[test]
fn a_plug_over_budget_is_refused_whole_not_trimmed() {
    let mut r = Rig::new(BLOCKS);
    r.want(3);
    assert_eq!(r.ask(PLUG, Rig::at(0), 4).kind, NACK);
    assert_eq!(r.plugged(), 0, "three of the four were not granted");
    assert_eq!(state(&mut r, 0, 4), ST_UNPLUGGED);
    assert_eq!(r.ask(PLUG, Rig::at(0), 3).kind, ACK);
    assert_eq!(r.t.device().counters().nacked, 1);
}

#[test]
fn a_refused_plug_can_be_retried_once_the_host_raises_the_size() {
    let mut r = Rig::new(BLOCKS);
    r.want(1);
    assert_eq!(r.ask(PLUG, Rig::at(0), 2).kind, NACK);
    r.want(2);
    assert_eq!(r.ask(PLUG, Rig::at(0), 2).kind, ACK);
    assert_eq!(r.plugged(), 2);
}

#[test]
fn a_plug_that_overlaps_a_held_block_is_an_error_and_changes_nothing() {
    let mut r = Rig::new(BLOCKS);
    r.want(8);
    assert_eq!(r.ask(PLUG, Rig::at(2), 2).kind, ACK);
    // Blocks 0..3 touch held block 2; the free blocks 0 and 1 must stay free.
    assert_eq!(r.ask(PLUG, Rig::at(0), 3).kind, ERROR);
    assert_eq!(r.ask(PLUG, Rig::at(3), 2).kind, ERROR);
    assert_eq!(r.ask(PLUG, Rig::at(2), 2).kind, ERROR);
    assert_eq!(r.plugged(), 2);
    assert_eq!(state(&mut r, 0, 2), ST_UNPLUGGED);
    assert_eq!(state(&mut r, 2, 2), ST_PLUGGED);
    assert_eq!(state(&mut r, 4, 1), ST_UNPLUGGED);
}

#[test]
fn an_overlap_is_an_error_even_when_the_budget_would_also_refuse_it() {
    let mut r = Rig::new(BLOCKS);
    r.want(2);
    assert_eq!(r.ask(PLUG, Rig::at(0), 2).kind, ACK);
    assert_eq!(
        r.ask(PLUG, Rig::at(0), 1).kind,
        ERROR,
        "double plug, not a nack"
    );
    assert_eq!(r.t.device().counters().nacked, 0);
}

#[test]
fn an_unplug_of_a_half_held_range_is_an_error_and_returns_nothing() {
    let mut r = Rig::new(BLOCKS);
    r.want(8);
    assert_eq!(r.ask(PLUG, Rig::at(2), 2).kind, ACK);
    assert_eq!(r.ask(UNPLUG, Rig::at(1), 3).kind, ERROR);
    assert_eq!(r.ask(UNPLUG, Rig::at(2), 3).kind, ERROR);
    assert_eq!(r.plugged(), 2);
    assert_eq!(state(&mut r, 2, 2), ST_PLUGGED);
    assert!(r.discards().is_empty());
}

#[test]
fn state_reports_a_range_as_one_of_exactly_three_things() {
    let mut r = Rig::new(BLOCKS);
    r.want(8);
    assert_eq!(r.ask(PLUG, Rig::at(4), 2).kind, ACK);
    assert_eq!(state(&mut r, 4, 2), ST_PLUGGED);
    assert_eq!(state(&mut r, 4, 1), ST_PLUGGED);
    assert_eq!(state(&mut r, 6, 4), ST_UNPLUGGED);
    assert_eq!(state(&mut r, 3, 2), ST_MIXED);
    assert_eq!(state(&mut r, 5, 2), ST_MIXED);
    assert_eq!(state(&mut r, 0, 16), ST_MIXED);
    // A query never changes anything.
    assert_eq!(r.plugged(), 2);
}

#[test]
fn lowering_the_request_never_takes_memory_back_but_blocks_further_plugs() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    assert_eq!(r.ask(PLUG, Rig::at(0), 4).kind, ACK);
    r.want(1);
    assert_eq!(r.plugged(), 4, "only the guest can give a block back");
    assert!(!r.t.device().is_quiescent());
    assert_eq!(r.ask(PLUG, Rig::at(8), 1).kind, NACK);
    assert_eq!(r.ask(UNPLUG, Rig::at(1), 3).kind, ACK);
    assert_eq!(r.plugged(), 1);
    assert_eq!(
        r.ask(PLUG, Rig::at(8), 1).kind,
        NACK,
        "1 held + 1 > 1 asked"
    );
    assert_eq!(r.ask(UNPLUG, Rig::at(0), 1).kind, ACK);
    assert!(r.t.device().is_quiescent());
    assert_eq!(r.ask(PLUG, Rig::at(8), 1).kind, ACK);
}

#[test]
fn a_host_that_cannot_take_a_block_back_leaves_it_plugged_and_says_so() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    assert_eq!(r.ask(PLUG, Rig::at(0), 4).kind, ACK);
    r.fail.store(true, Ordering::Relaxed);
    assert_eq!(r.ask(UNPLUG, Rig::at(0), 2).kind, ERROR);
    assert_eq!(r.ask(UNPLUG_ALL, 0, 0).kind, ERROR);
    assert_eq!(r.plugged(), 4, "no block was reported free that is not");
    assert_eq!(state(&mut r, 0, 4), ST_PLUGGED);
    let counters = r.t.device().counters();
    assert_eq!((counters.unplugged, counters.errored), (0, 2));
    // The host recovers; the same requests now succeed.
    r.fail.store(false, Ordering::Relaxed);
    assert_eq!(r.ask(UNPLUG, Rig::at(0), 2).kind, ACK);
    assert_eq!(r.plugged(), 2);
}

#[test]
fn memory_goes_back_to_the_host_as_region_relative_ranges_in_order() {
    let mut r = Rig::new(BLOCKS);
    r.want(8);
    assert_eq!(r.ask(PLUG, Rig::at(0), 8).kind, ACK);
    assert_eq!(r.ask(UNPLUG, Rig::at(5), 2).kind, ACK);
    assert_eq!(r.ask(UNPLUG, Rig::at(0), 1).kind, ACK);
    assert_eq!(r.ask(UNPLUG_ALL, 0, 0).kind, ACK);
    assert_eq!(
        r.discards(),
        vec![
            (5 * BLOCK_SIZE, 2 * BLOCK_SIZE),
            (0, BLOCK_SIZE),
            (0, BLOCKS * BLOCK_SIZE)
        ],
        "offsets are from the region start, never guest physical addresses"
    );
}

#[test]
fn counters_follow_the_script_and_never_count_a_refusal_as_success() {
    let mut r = Rig::new(BLOCKS);
    r.want(2);
    assert_eq!(r.ask(PLUG, Rig::at(0), 2).kind, ACK);
    assert_eq!(r.ask(PLUG, Rig::at(2), 1).kind, NACK);
    assert_eq!(r.ask(PLUG, Rig::at(0), 1).kind, ERROR);
    assert_eq!(r.ask(UNPLUG, Rig::at(0), 1).kind, ACK);
    assert_eq!(r.ask(UNPLUG, Rig::at(0), 1).kind, ERROR);
    assert_eq!(r.ask(99, 0, 0).kind, ERROR);
    let c = r.t.device().counters();
    assert_eq!(
        (c.plugged, c.unplugged, c.nacked, c.errored, c.rejected),
        (1, 1, 1, 3, 0)
    );
}

#[test]
fn the_host_is_asked_to_back_only_what_the_device_will_promise() {
    let mut r = Rig::new(BLOCKS);
    r.want(2);
    assert_eq!(r.ask(PLUG, Rig::at(3), 1).kind, ACK);
    // Refused for budget, for overlap, for range, for a bad type: none of them reach the host.
    assert_eq!(r.ask(PLUG, Rig::at(0), 2).kind, NACK);
    assert_eq!(r.ask(PLUG, Rig::at(3), 1).kind, ERROR);
    assert_eq!(r.ask(PLUG, Rig::at(BLOCKS), 1).kind, ERROR);
    assert_eq!(r.ask(PLUG, Rig::at(0) + 1, 1).kind, ERROR);
    assert_eq!(r.ask(99, Rig::at(0), 1).kind, ERROR);
    assert_eq!(r.backs(), vec![(3 * BLOCK_SIZE, BLOCK_SIZE)]);
}

#[test]
fn a_block_the_host_cannot_back_is_refused_and_never_promised() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    r.fail.store(true, Ordering::Relaxed);
    assert_eq!(r.ask(PLUG, Rig::at(0), 3).kind, ERROR);
    assert_eq!(r.plugged(), 0, "nothing was acknowledged, nothing is held");
    assert_eq!(state(&mut r, 0, 3), ST_UNPLUGGED);
    assert!(r.backs().is_empty());
    assert_eq!(r.t.device().counters().plugged, 0);
    r.fail.store(false, Ordering::Relaxed);
    assert_eq!(
        r.ask(PLUG, Rig::at(0), 3).kind,
        ACK,
        "the same request succeeds later"
    );
    assert_eq!(r.backs(), vec![(0, 3 * BLOCK_SIZE)]);
}
