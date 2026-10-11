//! A software guest driver that does what Linux's `virtio_mem` does: read the size the host wants,
//! plug blocks until the guest holds it, unplug the highest ones when it holds too much, back off
//! when told no, and keep its own idea of what it holds. Whatever the driver believes must match
//! the device, and the host must be asked to back and to take back exactly the same amounts.

use super::rig::{ACK, NACK, PLUG, Rig, ST_PLUGGED, STATE, UNPLUG};
use super::*;
use std::cmp::Ordering;

const BLOCKS: u64 = 32;
/// The widest plug the driver tries in one request.
const BURST: u16 = 4;

struct Driver {
    held: Vec<bool>,
    nacks: u32,
    busy: u32,
}

impl Driver {
    fn new() -> Self {
        Self {
            held: vec![false; usize::try_from(BLOCKS).expect("small")],
            nacks: 0,
            busy: 0,
        }
    }

    fn count(&self) -> u64 {
        self.held.iter().filter(|held| **held).count() as u64
    }

    /// One pass of the driver's work item: move toward the requested size. Returns whether it
    /// changed anything. A refusal ends the pass; the driver tries again on the next change.
    fn step(&mut self, r: &mut Rig) -> bool {
        let want = r.t.device().requested_size() / BLOCK_SIZE;
        match self.count().cmp(&want) {
            Ordering::Less => self.plug_more(r, want - self.count()),
            Ordering::Greater => self.unplug_one(r),
            Ordering::Equal => false,
        }
    }

    /// Plugs up to `short` blocks as one run starting at the first free block.
    fn plug_more(&mut self, r: &mut Rig, short: u64) -> bool {
        let first = self
            .held
            .iter()
            .position(|held| !held)
            .expect("a free block");
        let run = self.held[first..].iter().take_while(|held| !**held).count() as u64;
        let take = u16::try_from(short.min(run).min(u64::from(BURST))).expect("small");
        match r.ask(PLUG, Rig::at(first as u64), take).kind {
            ACK => {
                self.held[first..first + usize::from(take)].fill(true);
                true
            }
            NACK => {
                self.nacks += 1;
                false
            }
            2 => {
                self.busy += 1;
                false
            }
            other => panic!("the driver sent a bad plug: {other}"),
        }
    }

    fn unplug_one(&mut self, r: &mut Rig) -> bool {
        let last = self
            .held
            .iter()
            .rposition(|held| *held)
            .expect("a held block");
        assert_eq!(
            r.ask(UNPLUG, Rig::at(last as u64), 1).kind,
            ACK,
            "unplug of a held block"
        );
        self.held[last] = false;
        true
    }

    fn converge(&mut self, r: &mut Rig) {
        for _ in 0..(BLOCKS * 2) {
            if !self.step(r) {
                return;
            }
        }
        panic!("the driver never converged");
    }

    /// The driver and the device agree on every block, and the host's books balance.
    fn assert_consistent(&self, r: &mut Rig) {
        assert_eq!(self.count(), r.plugged(), "blocks held");
        for block in 0..BLOCKS {
            let held = r.ask(STATE, Rig::at(block), 1).state == ST_PLUGGED;
            assert_eq!(
                held,
                self.held[usize::try_from(block).expect("small")],
                "block {block}"
            );
        }
        let sum = |log: Vec<(u64, u64)>| log.iter().map(|(_, len)| len).sum::<u64>();
        assert_eq!(
            sum(r.backs()) - sum(r.discards()),
            r.plugged() * BLOCK_SIZE,
            "the host backs exactly what the guest holds"
        );
    }
}

#[test]
fn the_driver_plugs_up_to_the_request_and_no_further() {
    let mut r = Rig::new(BLOCKS);
    let mut d = Driver::new();
    r.want(10);
    d.converge(&mut r);
    assert_eq!(r.plugged(), 10);
    assert_eq!(
        d.nacks, 0,
        "a driver that stops at the target is never told no"
    );
    d.assert_consistent(&mut r);
}

#[test]
fn the_driver_gives_memory_back_when_the_request_falls() {
    let mut r = Rig::new(BLOCKS);
    let mut d = Driver::new();
    r.want(12);
    d.converge(&mut r);
    r.want(5);
    d.converge(&mut r);
    assert_eq!(r.plugged(), 5);
    d.assert_consistent(&mut r);
    r.want(0);
    d.converge(&mut r);
    assert!(r.t.device().is_quiescent());
    d.assert_consistent(&mut r);
}

#[test]
fn a_request_for_the_whole_region_and_back_to_nothing_balances() {
    let mut r = Rig::new(BLOCKS);
    let mut d = Driver::new();
    for target in [BLOCKS, 0, BLOCKS, 1, BLOCKS - 1, 0] {
        r.want(target);
        d.converge(&mut r);
        assert_eq!(r.plugged(), target);
        d.assert_consistent(&mut r);
    }
}

#[test]
fn an_over_eager_driver_is_told_no_and_settles_on_what_it_was_given() {
    let mut r = Rig::new(BLOCKS);
    r.want(2);
    // Four at once is more than the host asked for: refused, nothing granted.
    assert_eq!(r.ask(PLUG, Rig::at(0), 4).kind, NACK);
    assert_eq!(r.plugged(), 0);
    let mut d = Driver::new();
    d.converge(&mut r);
    assert_eq!(r.plugged(), 2);
    d.assert_consistent(&mut r);
}

#[test]
fn the_device_never_asks_the_driver_to_retry_later() {
    let mut r = Rig::new(BLOCKS);
    let mut d = Driver::new();
    for target in [7, 3, 20, 20, 1, 31, 0] {
        r.want(target);
        d.converge(&mut r);
    }
    assert_eq!(
        d.busy, 0,
        "BUSY is a response a driver must handle, this device never sends it"
    );
    assert_eq!(r.t.device().counters().errored, 0);
}

#[test]
fn a_long_random_walk_of_requests_leaves_the_books_balanced() {
    let mut r = Rig::new(BLOCKS);
    let mut d = Driver::new();
    let mut seed = 0x1234_5678_9abc_def0_u64;
    for _ in 0..300 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        r.want(seed % (BLOCKS + 1));
        d.converge(&mut r);
        assert_eq!(r.plugged(), r.t.device().requested_size() / BLOCK_SIZE);
    }
    d.assert_consistent(&mut r);
    assert_eq!(
        r.t.device().counters().errored,
        0,
        "a correct driver is never in error"
    );
}
