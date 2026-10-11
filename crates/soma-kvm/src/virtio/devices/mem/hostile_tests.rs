//! The requests a hostile guest sends: bad ranges, bad shapes, and random noise.

use super::tests::{BLOCKS, REGION, ask, block, boot, request, request_size};
use super::*;
use crate::virtio::devices::harness::Seg;
use crate::virtio::devices::service::service_queue;

#[test]
fn hostile_ranges_are_errors_and_change_nothing() {
    let mut r = boot();
    request_size(&mut r, 8 * BLOCK_SIZE);
    let cases = [
        (REQ_PLUG, block(0) + 4096, 1),           // unaligned
        (REQ_PLUG, REGION - BLOCK_SIZE, 1),       // below the region
        (REQ_PLUG, block(BLOCKS), 1),             // at the end
        (REQ_PLUG, block(BLOCKS - 1), 2),         // runs past the end
        (REQ_PLUG, block(0), 0),                  // empty
        (REQ_PLUG, block(0), u16::MAX),           // far too many
        (REQ_PLUG, u64::MAX - BLOCK_SIZE + 1, 4), // address overflow
        (REQ_UNPLUG, block(0), 1),                // never plugged
        (REQ_STATE, block(0), 0),
        (9, block(0), 1), // unknown request
    ];
    for (kind, addr, blocks) in cases {
        assert_eq!(
            ask(&mut r, kind, addr, blocks).0,
            RESP_ERROR,
            "{kind} {addr:#x} {blocks}"
        );
    }
    assert_eq!(r.t.device().plugged_size(), 0);
    assert!(r.discarded.lock().expect("log").is_empty());
    // A plug over a block that is already plugged is an error, not a second charge.
    assert_eq!(ask(&mut r, REQ_PLUG, block(0), 2), (RESP_ACK, 0));
    assert_eq!(ask(&mut r, REQ_PLUG, block(1), 2).0, RESP_ERROR);
    assert_eq!(r.t.device().plugged_size(), 2 * BLOCK_SIZE);
}

#[test]
fn malformed_chains_are_counted_and_answered_with_nothing() {
    let mut r = boot();
    // A request that is not 24 bytes, and one with nowhere to answer.
    for segments in [
        vec![
            Seg::readable(r.guest.alloc(&[0_u8; 16]), 16),
            Seg::writable(r.guest.alloc_zeroed(10), 10),
        ],
        vec![Seg::readable(
            r.guest.alloc(&request(REQ_STATE, block(0), 1)),
            24,
        )],
        vec![
            Seg::readable(r.guest.alloc(&request(REQ_STATE, block(0), 1)), 24),
            Seg::writable(r.guest.alloc_zeroed(4), 4),
        ],
    ] {
        r.guest.submit(0, &segments);
        r.guest.notify(&mut r.t, 0);
        let report = service_queue(&mut r.t, &r.guest.mem, 0, 8).expect("service");
        assert_eq!(report.completed, 1);
    }
    assert_eq!(r.t.device().counters().rejected, 3);
    assert_eq!(r.t.device().plugged_size(), 0);
}

#[test]
fn random_requests_never_fault_and_never_exceed_what_was_requested() {
    let mut r = boot();
    request_size(&mut r, 10 * BLOCK_SIZE);
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for round in 0..400_u64 {
        let kind = u16::try_from(next() % 5).expect("small");
        let addr = REGION + (next() % (BLOCKS + 4)) * BLOCK_SIZE;
        let blocks = u16::try_from(next() % 6).expect("small");
        let _ = ask(&mut r, kind, addr, blocks);
        if round % 97 == 0 {
            request_size(&mut r, (next() % 12) * BLOCK_SIZE);
        }
        let device = r.t.device();
        assert!(device.plugged_size() <= BLOCKS * BLOCK_SIZE);
        let counted: u64 = device
            .bitmap
            .iter()
            .map(|word| u64::from(word.count_ones()))
            .sum();
        assert_eq!(
            counted, device.plugged_blocks,
            "bitmap and count must agree"
        );
    }
}
