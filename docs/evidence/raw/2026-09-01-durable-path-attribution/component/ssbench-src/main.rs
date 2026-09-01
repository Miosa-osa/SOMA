//! Isolates the cost of one durable state write under N simultaneous callers.
use std::{
    sync::{Arc, Barrier},
    time::Instant,
};

use soma::{InstanceId, StateRecord, StateStore};
use soma_local::FileStateStore;

fn hex(n: u64, salt: u64) -> String {
    let mut s = String::new();
    let mut v = n
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(salt.wrapping_mul(0xBF58_476D_1CE4_E5B9));
    for _ in 0..32 {
        let d = (v & 0xF) as u32;
        s.push(char::from_digit(d, 16).unwrap());
        v = v.rotate_left(4) ^ (v >> 7).wrapping_mul(0x94D0_49BB_1331_11EB);
    }
    s
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = args[1].clone();
    let n: u64 = args[2].parse().unwrap();
    let rounds: u64 = args.get(3).map_or(1, |v| v.parse().unwrap());
    let record_len: usize = args.get(4).map_or(1200, |v| v.parse().unwrap());

    let mut all: Vec<u128> = Vec::new();
    for round in 0..rounds {
        let barrier = Arc::new(Barrier::new(n as usize));
        let mut handles = Vec::new();
        for i in 0..n {
            let root = root.clone();
            let barrier = Arc::clone(&barrier);
            let id = hex(i, round * 1_000_003 + 17);
            handles.push(std::thread::spawn(move || {
                let mut store = FileStateStore::open(&root).expect("open");
                let instance = InstanceId::new(id).expect("id");
                let record = StateRecord::from_bytes(vec![b'x'; record_len]).expect("record");
                barrier.wait();
                let started = Instant::now();
                store.create(&instance, record).expect("create");
                started.elapsed().as_nanos()
            }));
        }
        for h in handles {
            all.push(h.join().unwrap());
        }
    }
    all.sort_unstable();
    let pick = |q: f64| all[((all.len() as f64 - 1.0) * q).round() as usize] as f64 / 1e6;
    println!(
        "n={n} rounds={rounds} samples={} p50={:.2}ms p95={:.2}ms max={:.2}ms",
        all.len(),
        pick(0.5),
        pick(0.95),
        pick(1.0)
    );
    for v in &all {
        eprintln!("{v}");
    }
}
