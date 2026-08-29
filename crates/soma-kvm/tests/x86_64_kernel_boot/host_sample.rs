//! Single-sample host-side residency diagnostics for the live boot test.
//!
//! A sampler thread polls the test process while the guest runs and keeps the last sample taken
//! before the run returned and the sample with the highest `VmRSS`. The numbers are diagnostic
//! and single-sample; they are not a certified per-VM overhead figure.

use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const POLL: Duration = Duration::from_millis(2);

#[derive(Clone, Debug, Default)]
pub struct HostSample {
    pub vm_rss_kb: u64,
    pub rss_anon_kb: u64,
    pub rss_file_kb: u64,
    pub rss_shmem_kb: u64,
    pub threads: u64,
    pub fds: usize,
    /// `Rss` of the anonymous mapping whose size equals the guest RAM, when found.
    pub guest_mapping_rss_kb: Option<u64>,
    pub rollup: String,
}

fn field(text: &str, key: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix(key))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.parse().ok())
}

/// Finds the `Rss` of the first mapping in `/proc/self/smaps` with exactly `guest_ram_kb` size.
fn guest_mapping_rss(guest_ram_kb: u64) -> Option<u64> {
    let smaps = fs::read_to_string("/proc/self/smaps").ok()?;
    let mut in_target = false;
    for line in smaps.lines() {
        if line.contains('-') && line.split_whitespace().count() >= 5 {
            in_target = false;
            continue;
        }
        if let Some(rest) = line.strip_prefix("Size:") {
            let size: u64 = rest.split_whitespace().next()?.parse().ok()?;
            in_target = size == guest_ram_kb;
        } else if in_target && line.starts_with("Rss:") {
            return line
                .strip_prefix("Rss:")?
                .split_whitespace()
                .next()?
                .parse()
                .ok();
        }
    }
    None
}

pub fn sample(guest_ram_kb: u64) -> HostSample {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let rollup = fs::read_to_string("/proc/self/smaps_rollup")
        .unwrap_or_default()
        .lines()
        .filter(|line| {
            line.starts_with("Rss:")
                || line.starts_with("Pss:")
                || line.starts_with("Anonymous:")
                || line.starts_with("Private_Dirty:")
        })
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join(", ");
    HostSample {
        vm_rss_kb: field(&status, "VmRSS:").unwrap_or(0),
        rss_anon_kb: field(&status, "RssAnon:").unwrap_or(0),
        rss_file_kb: field(&status, "RssFile:").unwrap_or(0),
        rss_shmem_kb: field(&status, "RssShmem:").unwrap_or(0),
        threads: field(&status, "Threads:").unwrap_or(0),
        fds: fs::read_dir("/proc/self/fd").map_or(0, Iterator::count),
        guest_mapping_rss_kb: guest_mapping_rss(guest_ram_kb),
        rollup,
    }
}

/// Maximum thread and descriptor counts observed at any poll.
#[derive(Clone, Copy, Debug, Default)]
pub struct HostPeaks {
    pub threads: u64,
    pub fds: usize,
}

pub struct Sampler {
    stop: Arc<AtomicBool>,
    worker: JoinHandle<(HostSample, HostSample, HostPeaks)>,
}

impl Sampler {
    /// Starts polling; the sampler thread itself is included in the thread count.
    pub fn start(guest_ram_kb: u64) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut last_with_guest = sample(guest_ram_kb);
            let mut peak = last_with_guest.clone();
            let mut peaks = HostPeaks::default();
            while !flag.load(Ordering::Acquire) {
                thread::sleep(POLL);
                let current = sample(guest_ram_kb);
                peaks.threads = peaks.threads.max(current.threads);
                peaks.fds = peaks.fds.max(current.fds);
                if current.guest_mapping_rss_kb.is_some() {
                    last_with_guest = current.clone();
                }
                if current.vm_rss_kb > peak.vm_rss_kb {
                    peak = current;
                }
            }
            (last_with_guest, peak, peaks)
        });
        Self { stop, worker }
    }

    /// Returns the last sample taken while the guest mapping existed, the peak-`VmRSS` sample,
    /// and the maximum thread and descriptor counts seen.
    pub fn stop(self) -> (HostSample, HostSample, HostPeaks) {
        self.stop.store(true, Ordering::Release);
        self.worker.join().expect("sampler thread must not panic")
    }
}

pub fn describe(label: &str, sample: &HostSample) {
    eprintln!(
        "{label}: VmRSS={} kB RssAnon={} kB RssFile={} kB RssShmem={} kB threads={} fds={} guest_mapping_rss={:?} kB rollup=[{}]",
        sample.vm_rss_kb,
        sample.rss_anon_kb,
        sample.rss_file_kb,
        sample.rss_shmem_kb,
        sample.threads,
        sample.fds,
        sample.guest_mapping_rss_kb,
        sample.rollup
    );
}
