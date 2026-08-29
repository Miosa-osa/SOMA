//! Printing the warm timeline, the percentile loop, and scanning the published objects.
//!
//! Every number is a raw monotonic nanosecond offset from the first byte of the manifest
//! being read; nothing is averaged away, and the loop prints its own samples so the retained
//! evidence can be rebuilt from the log.

use std::{fs::File, io::Read as _, path::Path};

use soma_kvm::x86_64::{Milestone, SandboxEvidence};

/// The warm milestones, in the order a restore reaches them.
pub const WARM: [(Milestone, &str); 18] = [
    (Milestone::ValidateManifest, "validate manifest"),
    (Milestone::CreateVm, "create VM"),
    (Milestone::MapMemory, "map memory privately"),
    (Milestone::RegisterSlots, "register memory slots"),
    (Milestone::Platform, "irqchip, PIT, routes"),
    (Milestone::Devices, "devices restored"),
    (Milestone::Vcpu, "vCPU created"),
    (Milestone::VcpuRestored, "vCPU state restored"),
    (Milestone::Events, "eventfds and interrupt state"),
    (Milestone::LaunchPageMapped, "launch page slot mapped"),
    (Milestone::LaunchPageWritten, "fresh launch page written"),
    (Milestone::EventLoop, "device thread serving"),
    (Milestone::RunStart, "resume"),
    (Milestone::LaunchPageConsumed, "launch page consumed"),
    (Milestone::VsockConnected, "vsock connected"),
    (Milestone::Handshake, "handshake done"),
    (Milestone::LaunchPageRetired, "repair done"),
    (Milestone::Ready, "ready"),
];

/// The milestones after readiness.
pub const AFTER: [(Milestone, &str); 4] = [
    (Milestone::Execute, "execute done"),
    (Milestone::Shutdown, "shutdown acknowledged"),
    (Milestone::GuestExit, "guest exit"),
    (Milestone::Cleanup, "cleanup"),
];

/// Prints one restored Instance's warm timeline with deltas.
pub fn timeline(label: &str, evidence: &SandboxEvidence) {
    eprintln!("[{label}] WARM timeline (ns since the restore began; delta from previous):");
    let mut previous = 0_u64;
    for (milestone, name) in WARM.into_iter().chain(AFTER) {
        let Some(at) = evidence.at(milestone) else {
            eprintln!("  {name:<32} absent");
            continue;
        };
        eprintln!(
            "  {name:<32} {at:>14} {:>+14}",
            i128::from(at) - i128::from(previous)
        );
        previous = at;
    }
}

/// Sorted samples with their median and ninety-ninth percentile.
pub struct Percentiles {
    pub samples: Vec<u64>,
    pub p50: u64,
    pub p99: u64,
}

impl Percentiles {
    /// Nearest-rank percentiles over the raw samples; no interpolation, no averaging.
    #[must_use]
    pub fn of(mut samples: Vec<u64>) -> Self {
        samples.sort_unstable();
        let rank = |percentile: usize| {
            let index = (samples.len() * percentile).div_ceil(100).max(1) - 1;
            samples.get(index).copied().unwrap_or(0)
        };
        Self {
            p50: rank(50),
            p99: rank(99),
            samples,
        }
    }
}

/// Prints the percentile table for one milestone across the loop's iterations.
pub fn percentiles(label: &str, samples: Vec<u64>) {
    let percentiles = Percentiles::of(samples);
    eprintln!(
        "  {label:<32} n={} p50={} p99={} min={} max={} samples={:?}",
        percentiles.samples.len(),
        percentiles.p50,
        percentiles.p99,
        percentiles.samples.first().copied().unwrap_or(0),
        percentiles.samples.last().copied().unwrap_or(0),
        percentiles.samples,
    );
}

/// Reads a whole published object.
pub fn read(path: &Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    File::open(path)
        .expect("open the published object")
        .read_to_end(&mut bytes)
        .expect("read the published object");
    bytes
}

/// Counts how many times `needle` occurs in `haystack`.
#[must_use]
pub fn occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() || haystack.len() < needle.len() {
        return 0;
    }
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

/// Lowercase hex SHA-256 of a whole file.
pub fn digest(path: &Path) -> String {
    use sha2::{Digest as _, Sha256};

    let mut file = File::open(path).expect("open for hashing");
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    loop {
        let count = file.read(&mut buffer).expect("read for hashing");
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    hasher
        .finalize()
        .iter()
        .fold(String::new(), |mut hex, byte| {
            use std::fmt::Write as _;
            write!(hex, "{byte:02x}").unwrap();
            hex
        })
}

/// Flips one byte of a copy of `source` at `offset` and writes it to `target`.
pub fn tamper(source: &Path, target: &Path, offset: usize) {
    let mut bytes = read(source);
    assert!(offset < bytes.len(), "tamper offset is outside the object");
    bytes[offset] ^= 0x01;
    std::fs::write(target, bytes).expect("write the tampered object");
}
