//! A/B timing for the ring slot: the remainder it replaces, `RingSize::slot`, and a bare mask.
//!
//! `cargo bench -p soma-kernels --bench ring_slot` prints, per variant, the median and p99 of
//! nanoseconds per call over repeated trials. Sizes come from a runtime table the optimizer cannot
//! fold, as a queue's size does in the device. Record the output with the host, CPU and commit in
//! `benchmarks/kernels/candidates.toml`.

use std::hint::black_box;
use std::time::Instant;

use soma_kernels::ring::{RingSize, ring_slot_reference};

const CALLS: u32 = 1 << 22;
const TRIALS: usize = 41;
const SIZES: [u16; 8] = [256, 256, 1_024, 256, 128, 256, 1_024, 256];

fn time(mut call: impl FnMut(u16, usize) -> u16) -> (f64, f64) {
    let mut samples = Vec::with_capacity(TRIALS);
    for _ in 0..TRIALS {
        let mut index = 0_u16;
        let mut sum = 0_u32;
        let start = Instant::now();
        for n in 0..CALLS {
            sum = sum.wrapping_add(u32::from(call(black_box(index), (n as usize) & 7)));
            index = index.wrapping_add(1);
        }
        let elapsed = start.elapsed();
        black_box(sum);
        samples.push(elapsed.as_secs_f64() * 1e9 / f64::from(CALLS));
    }
    samples.sort_by(f64::total_cmp);
    (
        samples[samples.len() / 2],
        samples[samples.len() * 99 / 100],
    )
}

fn main() {
    let sizes: [u16; 8] = black_box(SIZES);
    let rings: [RingSize; 8] = sizes.map(|size| RingSize::new(size).expect("power of two"));
    let masks: [u16; 8] = sizes.map(|size| size - 1);
    let results = [
        (
            "remainder (before)",
            time(|index, k| ring_slot_reference(index, black_box(sizes[k]))),
        ),
        (
            "RingSize::slot (shipped)",
            time(|index, k| black_box(rings[k]).slot(index)),
        ),
        (
            "bare mask (lower bound)",
            time(|index, k| index & black_box(masks[k])),
        ),
    ];
    for (name, (median, p99)) in results {
        println!("{name:<26} median {median:.3} ns/call  p99 {p99:.3} ns/call");
    }
}
