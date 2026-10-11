//! A reference model of the memory device, driven in lockstep with the real one by thousands of
//! random requests: valid, hostile, and interleaved with the host changing its request, the host
//! failing to take memory back, and the guest rebooting.
//!
//! The model is written from the contract, not from the code: a request is valid when its range
//! is whole blocks inside the region and 1..=4096 long; a plug of a held block is an error; a
//! plug past the requested size is refused whole; an unplug needs every block held and a host
//! that takes the memory back. Any disagreement, in the answer or in what is held, fails.

use std::sync::atomic::Ordering;

use super::rig::{
    ACK, ERROR, NACK, PLUG, REGION, Rig, ST_MIXED, ST_PLUGGED, ST_UNPLUGGED, STATE, UNPLUG,
    UNPLUG_ALL,
};
use super::*;
use crate::virtio::devices::harness::GuestRig;

/// More than two bitmap words, so ranges cross a word boundary.
const BLOCKS: u64 = 133;
const MAX_COUNT: u64 = 4096;

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

#[derive(Default)]
struct Model {
    held: Vec<bool>,
    requested: u64,
    host_fails: bool,
}

impl Model {
    fn new() -> Self {
        Self {
            held: vec![false; usize::try_from(BLOCKS).expect("small")],
            ..Self::default()
        }
    }

    fn count(&self) -> u64 {
        self.held.iter().filter(|held| **held).count() as u64
    }

    /// The blocks `[addr, addr + count)` when the request is valid.
    fn range(addr: u64, count: u16) -> Option<std::ops::Range<usize>> {
        let offset = addr.checked_sub(REGION)?;
        let count = u64::from(count);
        let first = offset / BLOCK_SIZE;
        let valid =
            offset % BLOCK_SIZE == 0 && (1..=MAX_COUNT).contains(&count) && first + count <= BLOCKS;
        valid.then(|| {
            usize::try_from(first).expect("small")..usize::try_from(first + count).expect("small")
        })
    }

    fn apply(&mut self, kind: u16, addr: u64, count: u16) -> (u16, u16) {
        match kind {
            UNPLUG_ALL if self.host_fails => (ERROR, 0),
            UNPLUG_ALL => {
                self.held.fill(false);
                (ACK, 0)
            }
            PLUG | UNPLUG | STATE => match Self::range(addr, count) {
                None => (ERROR, 0),
                Some(range) => self.ranged(kind, range),
            },
            _ => (ERROR, 0),
        }
    }

    fn ranged(&mut self, kind: u16, range: std::ops::Range<usize>) -> (u16, u16) {
        let held = self.held[range.clone()]
            .iter()
            .filter(|held| **held)
            .count();
        let whole = range.len();
        match kind {
            PLUG if held > 0 => (ERROR, 0),
            PLUG if self.count() + whole as u64 > self.requested => (NACK, 0),
            PLUG if self.host_fails => (ERROR, 0),
            PLUG => {
                self.held[range].fill(true);
                (ACK, 0)
            }
            UNPLUG if held < whole || self.host_fails => (ERROR, 0),
            UNPLUG => {
                self.held[range].fill(false);
                (ACK, 0)
            }
            _ => match held {
                0 => (ACK, ST_UNPLUGGED),
                n if n == whole => (ACK, ST_PLUGGED),
                _ => (ACK, ST_MIXED),
            },
        }
    }
}

/// One random request: mostly valid, sometimes skewed, out of range, or the wrong size.
fn pick(rng: &mut XorShift) -> (u16, u64, u16) {
    let kind = match rng.below(20) {
        0..=7 => PLUG,
        8..=13 => UNPLUG,
        14 => UNPLUG_ALL,
        15..=18 => STATE,
        _ => u16::try_from(4 + rng.below(60)).expect("small"),
    };
    let first = rng.below(BLOCKS + 3);
    let mut addr = Rig::at(first);
    let mut count = u16::try_from(1 + rng.below(9)).expect("small");
    match rng.below(16) {
        0 => addr += 1 + rng.below(BLOCK_SIZE - 1),
        1 => count = 0,
        2 => count = u16::try_from(MAX_COUNT + rng.below(3)).expect("small"),
        3 => addr = rng.next(),
        4 => count = u16::try_from(rng.below(1 << 16)).expect("small"),
        _ => {}
    }
    (kind, addr, count)
}

fn reconcile(r: &mut Rig, m: &Model, seed: u64, step: usize) {
    assert_eq!(
        r.plugged(),
        m.count(),
        "seed {seed} step {step}: blocks held"
    );
    assert!(r.plugged() <= BLOCKS);
    assert_eq!(r.t.device().requested_size(), m.requested * BLOCK_SIZE);
}

fn run(seed: u64, steps: usize) {
    let mut rng = XorShift(seed);
    let mut r = Rig::new(BLOCKS);
    let mut m = Model::new();
    for step in 0..steps {
        match rng.below(40) {
            0 => {
                m.requested = rng.below(BLOCKS + 1);
                r.want(m.requested);
            }
            1 => {
                m.host_fails = !m.host_fails;
                r.fail.store(m.host_fails, Ordering::Relaxed);
            }
            2 => {
                // A reset the host cannot honour keeps blocks tracked and refuses the next
                // driver (race_tests covers it), so the model reboots only a host that works.
                m.host_fails = false;
                r.fail.store(false, Ordering::Relaxed);
                r.guest = GuestRig::new(&[16]);
                r.guest.init(&mut r.t, MEM_FEATURES);
                m.held.fill(false);
            }
            _ => {
                let (kind, addr, count) = pick(&mut rng);
                let want = m.apply(kind, addr, count);
                let got = r.ask(kind, addr, count);
                assert_eq!(
                    (got.kind, got.state),
                    want,
                    "seed {seed} step {step}: kind {kind} addr {addr:#x} count {count}"
                );
                assert_eq!(got.used, 10);
            }
        }
        reconcile(&mut r, &m, seed, step);
    }
    // Every single block agrees, not only the total.
    r.fail.store(false, Ordering::Relaxed);
    for block in 0..BLOCKS {
        let held = r.ask(STATE, Rig::at(block), 1).state == ST_PLUGGED;
        assert_eq!(
            held,
            m.held[usize::try_from(block).expect("small")],
            "seed {seed} block {block}"
        );
    }
}

#[test]
fn the_device_agrees_with_the_contract_over_random_scripts() {
    for seed in [
        1,
        2,
        3,
        0x9e37_79b9_7f4a_7c15,
        0xdead_beef_cafe_f00d,
        42,
        7919,
        123_456_789,
    ] {
        run(seed, 2500);
    }
}

#[test]
fn a_greedy_guest_is_never_granted_past_the_size_asked_at_that_moment() {
    let mut r = Rig::new(BLOCKS);
    let mut rng = XorShift(0xfeed_face);
    let mut granted = 0_u32;
    for step in 0..1500 {
        if step % 25 == 0 {
            r.want(rng.below(BLOCKS + 1));
        }
        let first = rng.below(BLOCKS - 4);
        let count = u16::try_from(1 + rng.below(4)).expect("small");
        let kind = if rng.below(2) == 0 { UNPLUG } else { PLUG };
        let answer = r.ask(kind, Rig::at(first), count);
        if kind == PLUG && answer.kind == ACK {
            granted += 1;
            assert!(
                r.t.device().plugged_size() <= r.t.device().requested_size(),
                "step {step}: a grant took the guest past what the host asked for"
            );
        }
    }
    assert!(granted > 20, "the script exercised grants ({granted})");
}
