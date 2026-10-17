//! What the pool keeps of a guest's reads, and what it refuses to keep.
//!
//! These tests touch only the warm sets, and every one of them uses a key no other test names, so
//! they need no share of the pool's own serial guard.

use soma_kvm::head_reads::{HeadSpan, MAX_WARM_BYTES};

use super::{PrimedKey, note_head_reads, warm_for};

fn key(reference: &str) -> PrimedKey {
    PrimedKey {
        reference: reference.to_owned(),
        generation_id: soma::GenerationId::new(format!("sha256:{}", "ab".repeat(32)))
            .expect("a generation identity"),
        memory_mib: 2048,
    }
}

#[test]
fn a_warm_set_belongs_to_its_own_generation_and_no_other() {
    let mine = key("warm-set-own");
    let other = key("warm-set-other");
    note_head_reads(
        &mine,
        &[HeadSpan {
            offset: 0x8000_0000,
            len: 4096,
        }],
    );
    assert_eq!(
        warm_for(&mine),
        vec![HeadSpan {
            offset: 0x8000_0000,
            len: 4096
        }],
        "what this Generation's guest read is kept for this Generation"
    );
    assert!(
        warm_for(&other).is_empty(),
        "no Generation's observation can widen another's set"
    );
}

#[test]
fn a_later_larger_observation_cannot_grow_the_set_past_the_bound() {
    let bounded = key("warm-set-bound");
    note_head_reads(
        &bounded,
        &[HeadSpan {
            offset: 0,
            len: MAX_WARM_BYTES,
        }],
    );
    // A second host reports a range of its own, on top of a set already at the bound.
    note_head_reads(
        &bounded,
        &[HeadSpan {
            offset: MAX_WARM_BYTES * 8,
            len: 4096,
        }],
    );
    let total: u64 = warm_for(&bounded).iter().map(|span| span.len).sum();
    assert_eq!(
        total, MAX_WARM_BYTES,
        "the bound holds across every observation, not only the first"
    );
}
