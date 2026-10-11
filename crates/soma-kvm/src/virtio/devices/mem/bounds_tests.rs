//! Every edge of the region: the first and last block, one byte or one block outside, counts at
//! and past the limit, addresses that overflow, and sizes the host may not ask for.

use super::rig::{ACK, ERROR, PLUG, REGION, Rig, STATE, UNPLUG, UNPLUG_ALL};
use super::*;
use crate::virtio::devices::mem::backing::{RecordingBacking, RegionBacking};

const BLOCKS: u64 = 16;

fn backing() -> Box<dyn RegionBacking + Send> {
    Box::new(RecordingBacking::default())
}

#[test]
fn a_region_that_cannot_be_tracked_or_addressed_is_refused() {
    let region = 4 * BLOCK_SIZE;
    assert!(MemDevice::new(REGION, region, backing()).is_ok());
    assert_eq!(
        MemDevice::new(REGION, 0, backing()).err(),
        Some(SizeError::Unaligned)
    );
    assert_eq!(
        MemDevice::new(REGION + 1, region, backing()).err(),
        Some(SizeError::Unaligned)
    );
    assert_eq!(
        MemDevice::new(REGION, region + 1, backing()).err(),
        Some(SizeError::Unaligned)
    );
    assert_eq!(
        MemDevice::new(REGION, region - 1, backing()).err(),
        Some(SizeError::Unaligned)
    );
    let top = 1_u64 << 20;
    assert!(MemDevice::new(REGION, top * BLOCK_SIZE, backing()).is_ok());
    assert_eq!(
        MemDevice::new(REGION, (top + 1) * BLOCK_SIZE, backing()).err(),
        Some(SizeError::BeyondRegion)
    );
    // The end of the region would wrap the 64-bit address space.
    let wrapping = u64::MAX - u64::MAX % BLOCK_SIZE - BLOCK_SIZE;
    assert_eq!(
        MemDevice::new(wrapping, 4 * BLOCK_SIZE, backing()).err(),
        Some(SizeError::BeyondRegion)
    );
}

#[test]
fn the_first_and_last_block_work_and_the_neighbours_do_not() {
    let mut r = Rig::new(BLOCKS);
    r.want(BLOCKS);
    assert_eq!(r.ask(PLUG, Rig::at(0), 1).kind, ACK);
    assert_eq!(r.ask(PLUG, Rig::at(BLOCKS - 1), 1).kind, ACK);
    assert_eq!(r.plugged(), 2);
    // One block past the end, and one block before the start.
    assert_eq!(r.ask(PLUG, Rig::at(BLOCKS), 1).kind, ERROR);
    assert_eq!(r.ask(PLUG, REGION - BLOCK_SIZE, 1).kind, ERROR);
    assert_eq!(r.ask(UNPLUG, Rig::at(BLOCKS), 1).kind, ERROR);
    assert_eq!(r.ask(UNPLUG, REGION - BLOCK_SIZE, 1).kind, ERROR);
    assert_eq!(r.plugged(), 2);
    assert_eq!(r.ask(UNPLUG, Rig::at(BLOCKS - 1), 1).kind, ACK);
    assert_eq!(r.ask(UNPLUG, Rig::at(0), 1).kind, ACK);
    assert_eq!(r.plugged(), 0);
}

#[test]
fn an_address_off_the_block_grid_is_refused_by_any_amount() {
    let mut r = Rig::new(BLOCKS);
    r.want(BLOCKS);
    for skew in [1, 2, 511, 4096, BLOCK_SIZE - 1, BLOCK_SIZE + 1] {
        for kind in [PLUG, UNPLUG, STATE] {
            let addr = Rig::at(2) + skew % BLOCK_SIZE;
            assert_eq!(r.ask(kind, addr, 1).kind, ERROR, "kind {kind} skew {skew}");
        }
    }
    assert_eq!(r.plugged(), 0);
    assert!(r.discards().is_empty());
}

#[test]
fn addresses_that_overflow_or_sit_far_outside_are_refused() {
    let mut r = Rig::new(BLOCKS);
    r.want(BLOCKS);
    let grid_top = u64::MAX - u64::MAX % BLOCK_SIZE;
    for addr in [0, 1, BLOCK_SIZE, u64::MAX, grid_top, grid_top - BLOCK_SIZE] {
        for kind in [PLUG, UNPLUG, STATE] {
            assert_eq!(
                r.ask(kind, addr, 1).kind,
                ERROR,
                "kind {kind} addr {addr:#x}"
            );
            assert_eq!(r.ask(kind, addr, u16::MAX).kind, ERROR);
        }
    }
    assert_eq!(r.plugged(), 0);
}

#[test]
fn a_range_that_runs_off_the_end_changes_nothing() {
    let mut r = Rig::new(BLOCKS);
    r.want(BLOCKS);
    // BLOCKS - 2 + 3 reaches one block past the region.
    assert_eq!(r.ask(PLUG, Rig::at(BLOCKS - 2), 3).kind, ERROR);
    assert_eq!(r.plugged(), 0, "the in-range part was not plugged either");
    assert_eq!(r.ask(PLUG, Rig::at(BLOCKS - 2), 2).kind, ACK);
    assert_eq!(r.ask(UNPLUG, Rig::at(BLOCKS - 2), 3).kind, ERROR);
    assert_eq!(r.plugged(), 2, "the in-range part was not unplugged either");
    assert!(r.discards().is_empty(), "nothing went back to the host");
}

#[test]
fn counts_of_zero_and_past_the_request_limit_are_refused() {
    let mut r = Rig::new(8192);
    r.want(8192);
    assert_eq!(r.ask(PLUG, Rig::at(0), 0).kind, ERROR);
    assert_eq!(r.ask(STATE, Rig::at(0), 0).kind, ERROR);
    assert_eq!(r.ask(PLUG, Rig::at(0), u16::MAX).kind, ERROR);
    assert_eq!(r.ask(PLUG, Rig::at(0), 4097).kind, ERROR);
    assert_eq!(r.plugged(), 0);
    // Exactly the limit is a legal request on a region that holds it.
    assert_eq!(r.ask(PLUG, Rig::at(0), 4096).kind, ACK);
    assert_eq!(r.plugged(), 4096);
    assert_eq!(r.ask(UNPLUG, Rig::at(0), 4097).kind, ERROR);
    assert_eq!(r.ask(UNPLUG, Rig::at(0), 4096).kind, ACK);
    assert_eq!(r.discards(), vec![(0, 4096 * BLOCK_SIZE)]);
}

#[test]
fn unplug_all_ignores_the_address_and_count_it_is_given() {
    let mut r = Rig::new(BLOCKS);
    r.want(4);
    assert_eq!(r.ask(PLUG, Rig::at(0), 4).kind, ACK);
    assert_eq!(r.ask(UNPLUG_ALL, u64::MAX, u16::MAX).kind, ACK);
    assert_eq!(r.plugged(), 0);
    assert_eq!(r.discards(), vec![(0, BLOCKS * BLOCK_SIZE)]);
}

#[test]
fn the_host_may_only_ask_for_whole_blocks_inside_the_region() {
    let mut r = Rig::new(BLOCKS);
    let device = r.t.device_mut();
    assert_eq!(device.set_requested_size(BLOCKS * BLOCK_SIZE), Ok(()));
    assert_eq!(device.set_requested_size(0), Ok(()));
    for bad in [1, BLOCK_SIZE - 1, BLOCK_SIZE + 1, u64::MAX] {
        assert_eq!(device.set_requested_size(bad), Err(SizeError::Unaligned));
    }
    for bad in [(BLOCKS + 1) * BLOCK_SIZE, (BLOCKS + 1000) * BLOCK_SIZE] {
        assert_eq!(device.set_requested_size(bad), Err(SizeError::BeyondRegion));
    }
    assert_eq!(device.requested_size(), 0, "a refused size changes nothing");
}
