//! Isolates the private-head clone from the launch path so its cost can be ablated.
//!
//! One cohort releases `--threads` threads through a barrier; each performs exactly the
//! sequence `crates/soma-local/src/backend/kvm/boot.rs` performs for a writable Instance:
//! exclusive create under a directory descriptor, `FICLONE` from the overlay template,
//! `FIEMAP` verification that every extent is shared, and unlink. Every phase is timed
//! separately and each phase can be switched off, so a cost can be moved rather than only
//! correlated with.
//!
//! `--template` and `--dir` take comma separated lists and thread `i` uses entry `i % len`,
//! so independent copies and directories spread the work over distinct filesystem objects.
//!
//! Every cohort line carries the one minute load average taken as the cohort was released,
//! because this host is shared and a number taken under load is not evidence of anything.

#![allow(unsafe_code)]
#![allow(clippy::print_stdout)]

mod clone;

use std::io::Write as _;
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::sync::Barrier;
use std::time::Instant;

use clone::{Plan, Sample};

fn arg(name: &str, fallback: &str) -> String {
    let mut args = std::env::args().skip(1);
    while let Some(item) = args.next() {
        if item == name {
            return args.next().unwrap_or_else(|| fallback.to_owned());
        }
    }
    fallback.to_owned()
}

fn on(name: &str, fallback: bool) -> bool {
    matches!(
        arg(name, if fallback { "on" } else { "off" }).as_str(),
        "on" | "true" | "1"
    )
}

/// The one minute load average, so every cohort can be checked against host conditions.
fn load() -> f64 {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|text| text.split_whitespace().next()?.parse().ok())
        .unwrap_or(-1.0)
}

fn percentile(sorted: &[f64], point: usize) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (point * sorted.len()).div_ceil(100).max(1);
    sorted[rank.min(sorted.len()) - 1]
}

fn pair(samples: &[Sample], pick: fn(&Sample) -> f64) -> (f64, f64) {
    let mut values: Vec<f64> = samples.iter().map(pick).collect();
    values.sort_by(f64::total_cmp);
    (percentile(&values, 50), percentile(&values, 99))
}

fn main() {
    let plan = Plan {
        threads: arg("--threads", "100").parse().expect("threads"),
        cohorts: arg("--cohorts", "20").parse().expect("cohorts"),
        do_clone: on("--clone", true),
        do_verify: on("--verify", true),
        private_sources: on("--private-sources", false),
    };
    let dir_path = arg("--dir", "/srv/soma/heads");
    let gap_ms: u64 = arg("--gap-ms", "200").parse().expect("gap");
    let label = arg("--label", "run");
    let out = arg("--out", "");

    for path in dir_path.split(',') {
        std::fs::create_dir_all(path).expect("head dir");
    }
    let dirs = clone::open_all(&dir_path);
    let templates = clone::open_all(&arg("--template", ""));
    let mut sink = (!out.is_empty()).then(|| std::fs::File::create(&out).expect("out"));

    // Built once: rebuilding it inside the loop would charge each cohort for a hundred extra
    // reflinks and confound the very serialization the pool exists to remove.
    let pool = plan
        .private_sources
        .then(|| clone::source_pool(templates[0].as_raw_fd(), dirs[0].as_raw_fd(), plan.threads));

    let mut all: Vec<f64> = Vec::new();
    for cohort in 0..plan.cohorts {
        let barrier = Barrier::new(plan.threads);
        let before = load();
        let wall = Instant::now();
        let (samples, held): (Vec<Sample>, Vec<OwnedFd>) = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..plan.threads)
                .map(|index| {
                    let barrier = &barrier;
                    let pool = pool.as_ref();
                    let dirs = &dirs;
                    let templates = &templates;
                    scope.spawn(move || {
                        let dir = dirs[index % dirs.len()].as_raw_fd();
                        let spread = templates[index % templates.len()].as_raw_fd();
                        let source = pool.map_or(spread, |ready| ready[index].as_raw_fd());
                        barrier.wait();
                        clone::one(dir, source, index, &plan)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("join")).unzip()
        });
        let cohort_us = wall.elapsed().as_secs_f64() * 1e6;
        let teardown = Instant::now();
        drop(held);
        let teardown_us = teardown.elapsed().as_secs_f64() * 1e6;
        let after = load();

        let mut totals: Vec<f64> = samples.iter().map(Sample::total).collect();
        totals.sort_by(f64::total_cmp);
        all.extend_from_slice(&totals);
        let (create50, _) = pair(&samples, |s| s.create);
        let (clone50, clone99) = pair(&samples, |s| s.clone);
        let (verify50, verify99) = pair(&samples, |s| s.verify);
        let (unlink50, _) = pair(&samples, |s| s.unlink);
        println!(
            "{{\"label\":\"{label}\",\"cohort\":{cohort},\"threads\":{},\"load_before\":{before},\
             \"load_after\":{after},\"total_p50_us\":{:.1},\"total_p99_us\":{:.1},\
             \"create_p50_us\":{create50:.1},\"clone_p50_us\":{clone50:.1},\
             \"clone_p99_us\":{clone99:.1},\"verify_p50_us\":{verify50:.1},\
             \"verify_p99_us\":{verify99:.1},\"unlink_p50_us\":{unlink50:.1},\
             \"cohort_wall_us\":{cohort_us:.1},\"teardown_us\":{teardown_us:.1},\
             \"extents\":{}}}",
            plan.threads,
            percentile(&totals, 50),
            percentile(&totals, 99),
            samples[0].extents
        );
        if let Some(sink) = sink.as_mut() {
            for (index, sample) in samples.iter().enumerate() {
                writeln!(
                    sink,
                    "{{\"label\":\"{label}\",\"cohort\":{cohort},\"thread\":{index},\
                     \"load_before\":{before},\"create_us\":{:.1},\"clone_us\":{:.1},\
                     \"verify_us\":{:.1},\"unlink_us\":{:.1}}}",
                    sample.create, sample.clone, sample.verify, sample.unlink
                )
                .expect("write");
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(gap_ms));
    }
    all.sort_by(f64::total_cmp);
    println!(
        "{{\"label\":\"{label}\",\"summary\":true,\"samples\":{},\"p50_us\":{:.1},\
         \"p95_us\":{:.1},\"p99_us\":{:.1},\"max_us\":{:.1}}}",
        all.len(),
        percentile(&all, 50),
        percentile(&all, 95),
        percentile(&all, 99),
        all.last().copied().unwrap_or_default()
    );
}
