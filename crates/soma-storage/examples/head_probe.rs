//! Isolates the private-head clone from the launch path so its cost can be ablated.
//!
//! One cohort releases `--threads` threads through a barrier; each performs exactly the
//! sequence `crates/soma-local/src/backend/kvm/boot.rs` performs for a writable Instance:
//! exclusive create under a directory descriptor, `FICLONE` from the overlay template,
//! `FIEMAP` verification that every extent is shared, unlink, and close. Every phase is
//! timed separately and each phase can be switched off, so a cost can be moved rather than
//! only correlated with.
//!
//! `--template` takes a comma separated list and thread `i` clones from entry `i % len`, so
//! independent physical copies spread the clones over distinct allocation groups.

#![allow(unsafe_code)]
#![allow(clippy::print_stdout)]

use std::ffi::CString;
use std::io::Write as _;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd, RawFd};
use std::sync::Barrier;
use std::time::Instant;

/// One thread's timings for one cohort, in microseconds.
#[derive(Clone, Copy, Default)]
struct Sample {
    create: f64,
    clone: f64,
    verify: f64,
    unlink: f64,
    close: f64,
    extents: u64,
}

/// Which phases run.
#[derive(Clone, Copy)]
struct Plan {
    threads: usize,
    cohorts: usize,
    do_clone: bool,
    do_verify: bool,
    flag_sync: bool,
    private_sources: bool,
    hold: bool,
}

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

const FS_IOC_FIEMAP: libc::Ioctl = 0xC020_660B;
const FIEMAP_FLAG_SYNC: u32 = 0x1;
const FIEMAP_EXTENT_LAST: u32 = 0x1;
const FIEMAP_EXTENT_SHARED: u32 = 0x2000;
const BATCH: usize = 64;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Extent {
    logical: u64,
    physical: u64,
    length: u64,
    reserved64: [u64; 2],
    flags: u32,
    reserved: [u32; 3],
}

#[repr(C)]
struct Request {
    start: u64,
    length: u64,
    flags: u32,
    mapped: u32,
    count: u32,
    reserved: u32,
    extents: [Extent; BATCH],
}

/// Walks every extent and returns the count, failing if any extent is not shared.
fn verify(fd: RawFd, flag_sync: bool) -> u64 {
    let mut start = 0u64;
    let mut seen = 0u64;
    loop {
        let mut request = Request {
            start,
            length: u64::MAX,
            flags: if flag_sync { FIEMAP_FLAG_SYNC } else { 0 },
            mapped: 0,
            count: BATCH as u32,
            reserved: 0,
            extents: [Extent::default(); BATCH],
        };
        // SAFETY: `request` matches `struct fiemap` followed by `count` extents and outlives
        // the call; `fd` is live.
        let rc = unsafe { libc::ioctl(fd, FS_IOC_FIEMAP, &raw mut request) };
        assert!(rc == 0, "fiemap: {}", std::io::Error::last_os_error());
        let mapped = (request.mapped as usize).min(BATCH);
        if mapped == 0 {
            return seen;
        }
        for extent in &request.extents[..mapped] {
            assert!(extent.flags & FIEMAP_EXTENT_SHARED != 0, "extent not shared");
            seen += 1;
        }
        let last = request.extents[mapped - 1];
        if last.flags & FIEMAP_EXTENT_LAST != 0 {
            return seen;
        }
        start = last.logical + last.length;
    }
}

fn create_exclusive(dir: RawFd, name: &CString) -> OwnedFd {
    let flags = libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW;
    // SAFETY: `name` is NUL terminated and outlives the call and `dir` is a live descriptor.
    let fd = unsafe { libc::openat(dir, name.as_ptr(), flags, 0o600 as libc::c_uint) };
    assert!(fd >= 0, "openat: {}", std::io::Error::last_os_error());
    // SAFETY: `fd` was just returned by `openat` and is owned by nobody else.
    unsafe { OwnedFd::from_raw_fd(fd) }
}

fn unlink(dir: RawFd, name: &CString) {
    // SAFETY: `name` is NUL terminated and outlives the call and `dir` is a live descriptor.
    let rc = unsafe { libc::unlinkat(dir, name.as_ptr(), 0) };
    assert!(rc == 0, "unlinkat: {}", std::io::Error::last_os_error());
}

fn open_ro(path: &str) -> OwnedFd {
    let file = std::fs::File::open(path).unwrap_or_else(|error| panic!("open {path}: {error}"));
    OwnedFd::from(file)
}

/// Reflinks `count` private copies of the template so no two threads share a source inode.
fn source_pool(template: RawFd, dir: RawFd, count: usize) -> Vec<OwnedFd> {
    (0..count)
        .map(|index| {
            let name = CString::new(format!("srcpool-{index}")).expect("name");
            let fd = create_exclusive(dir, &name);
            // SAFETY: both descriptors are live for the call.
            let rc = unsafe { libc::ioctl(fd.as_raw_fd(), libc::FICLONE, template) };
            assert!(rc == 0, "pool ficlone: {}", std::io::Error::last_os_error());
            unlink(dir, &name);
            fd
        })
        .collect()
}

fn one(dir: RawFd, source: RawFd, index: usize, plan: &Plan) -> (Sample, Option<OwnedFd>) {
    let mut sample = Sample::default();
    let name = CString::new(format!("probe-{index:04}")).expect("name");
    let mark = Instant::now();
    let fd = create_exclusive(dir, &name);
    sample.create = mark.elapsed().as_secs_f64() * 1e6;
    if plan.do_clone {
        let mark = Instant::now();
        // SAFETY: both descriptors are live for the duration of the call.
        let rc = unsafe { libc::ioctl(fd.as_raw_fd(), libc::FICLONE, source) };
        assert!(rc == 0, "ficlone: {}", std::io::Error::last_os_error());
        sample.clone = mark.elapsed().as_secs_f64() * 1e6;
    }
    if plan.do_verify && plan.do_clone {
        let mark = Instant::now();
        sample.extents = verify(fd.as_raw_fd(), plan.flag_sync);
        sample.verify = mark.elapsed().as_secs_f64() * 1e6;
    }
    let mark = Instant::now();
    unlink(dir, &name);
    sample.unlink = mark.elapsed().as_secs_f64() * 1e6;
    if plan.hold {
        return (sample, Some(fd));
    }
    let mark = Instant::now();
    drop(fd);
    sample.close = mark.elapsed().as_secs_f64() * 1e6;
    (sample, None)
}

fn percentile(sorted: &[f64], point: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((point / 100.0) * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted[rank.min(sorted.len()) - 1]
}

fn total(sample: &Sample) -> f64 {
    sample.create + sample.clone + sample.verify + sample.unlink + sample.close
}

fn pair(samples: &[Sample], pick: fn(&Sample) -> f64) -> (f64, f64) {
    let mut values: Vec<f64> = samples.iter().map(pick).collect();
    values.sort_by(f64::total_cmp);
    (percentile(&values, 50.0), percentile(&values, 99.0))
}

fn main() {
    let template_paths = arg("--template", "");
    let dir_path = arg("--dir", "/srv/soma/heads");
    let plan = Plan {
        threads: arg("--threads", "100").parse().expect("threads"),
        cohorts: arg("--cohorts", "20").parse().expect("cohorts"),
        do_clone: on("--clone", true),
        do_verify: on("--verify", true),
        flag_sync: on("--flag-sync", true),
        private_sources: on("--private-sources", false),
        hold: on("--hold", true),
    };
    let gap_ms: u64 = arg("--gap-ms", "200").parse().expect("gap");
    let label = arg("--label", "run");
    let out = arg("--out", "");

    std::fs::create_dir_all(&dir_path).expect("head dir");
    let dir = open_ro(&dir_path);
    let templates: Vec<OwnedFd> = template_paths.split(',').map(open_ro).collect();
    let mut sink = (!out.is_empty()).then(|| std::fs::File::create(&out).expect("out"));

    let mut all: Vec<f64> = Vec::new();
    // Built once: rebuilding it inside the loop would charge each cohort for a hundred extra
    // reflinks and confound the very serialization the pool exists to remove.
    let pool = plan
        .private_sources
        .then(|| source_pool(templates[0].as_raw_fd(), dir.as_raw_fd(), plan.threads));
    for cohort in 0..plan.cohorts {
        let barrier = Barrier::new(plan.threads);
        let wall = Instant::now();
        let (samples, held): (Vec<Sample>, Vec<Option<OwnedFd>>) = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..plan.threads)
                .map(|index| {
                    let barrier = &barrier;
                    let pool = pool.as_ref();
                    let dir = dir.as_raw_fd();
                    let templates = &templates;
                    scope.spawn(move || {
                        let spread = templates[index % templates.len()].as_raw_fd();
                        let source = pool.map_or(spread, |ready| ready[index].as_raw_fd());
                        barrier.wait();
                        one(dir, source, index, &plan)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("join")).unzip()
        });
        let cohort_us = wall.elapsed().as_secs_f64() * 1e6;
        let teardown = Instant::now();
        drop(held);
        let teardown_us = teardown.elapsed().as_secs_f64() * 1e6;

        let mut totals: Vec<f64> = samples.iter().map(total).collect();
        totals.sort_by(f64::total_cmp);
        all.extend_from_slice(&totals);
        let (create50, _) = pair(&samples, |s| s.create);
        let (clone50, clone99) = pair(&samples, |s| s.clone);
        let (verify50, verify99) = pair(&samples, |s| s.verify);
        let (unlink50, _) = pair(&samples, |s| s.unlink);
        let (close50, _) = pair(&samples, |s| s.close);
        println!(
            "{{\"label\":\"{label}\",\"cohort\":{cohort},\"threads\":{},\"total_p50_us\":{:.1},\
             \"total_p99_us\":{:.1},\"create_p50_us\":{create50:.1},\"clone_p50_us\":{clone50:.1},\
             \"clone_p99_us\":{clone99:.1},\"verify_p50_us\":{verify50:.1},\
             \"verify_p99_us\":{verify99:.1},\"unlink_p50_us\":{unlink50:.1},\
             \"close_p50_us\":{close50:.1},\"cohort_wall_us\":{cohort_us:.1},\
             \"teardown_us\":{teardown_us:.1},\"extents\":{}}}",
            plan.threads,
            percentile(&totals, 50.0),
            percentile(&totals, 99.0),
            samples[0].extents
        );
        if let Some(sink) = sink.as_mut() {
            for (index, sample) in samples.iter().enumerate() {
                writeln!(
                    sink,
                    "{{\"label\":\"{label}\",\"cohort\":{cohort},\"thread\":{index},\
                     \"create_us\":{:.1},\"clone_us\":{:.1},\"verify_us\":{:.1},\
                     \"unlink_us\":{:.1},\"close_us\":{:.1}}}",
                    sample.create, sample.clone, sample.verify, sample.unlink, sample.close
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
        percentile(&all, 50.0),
        percentile(&all, 95.0),
        percentile(&all, 99.0),
        all.last().copied().unwrap_or_default()
    );
}
