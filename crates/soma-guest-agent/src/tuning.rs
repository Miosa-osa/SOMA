//! Guest tuning for the large shape, applied before the capture point.
//!
//! Machine contract v2 admits eight processors and sixteen gigabytes, which is the shape a build
//! workload runs in: a package install writes a hundred thousand small files and a typecheck
//! reads them all back. Three settings turn that from "correct" into "fast", and all three are
//! guest state, so they are applied at boot and captured with the machine. Every restored
//! Instance therefore starts already tuned instead of running a tuning script of its own.
//!
//! Nothing here is a security boundary and nothing here fails a boot. A guest whose kernel was
//! built without swap, or whose image carries no `mkswap`, must still start: tuning is a
//! performance claim, and a machine that refuses to run because it could not make itself faster
//! has traded a working sandbox for a measurement. So every step reports what it did, and the
//! report is one console line the boot evidence carries.
//!
//! The two sizing steps are gated on [`LARGE_FLOOR_BYTES`], so a version 1 machine is untouched by
//! them: its writeback thresholds and its swap devices stay exactly what the kernel chose at boot.

#![allow(clippy::module_name_repetitions)]

use std::{fmt, fs, path::Path};

mod hardening;
mod writeback;
mod zram;

pub use hardening::clear_boot_log;
pub use zram::Zram;

/// The guest RAM above which the large-only sizing steps apply.
///
/// Machine contract v1 caps RAM at 3 GiB (`ram-max=3221225472`) and contract v2 raises that to
/// 16 GiB, so a machine whose total memory is above the version 1 ceiling can only be a version 2
/// machine. This is the contract expressed as the one number the guest can read for itself, which
/// keeps a version 1 machine on exactly the kernel defaults it has always run with.
pub const LARGE_FLOOR_BYTES: u64 = 3 * 1024 * 1024 * 1024;

/// The writeback cap never exceeds this, however much RAM the machine has.
///
/// A ratio is a percentage of available memory, and available memory shrinks exactly when the
/// machine is tight. A byte cap bounds the worst case on every machine size, which is why the cap
/// is a byte count and the two ratios are forced to zero beside it.
const DIRTY_CAP_CEILING_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Guest RAM is divided by this to size the writeback cap.
const DIRTY_CAP_DIVISOR: u64 = 4;
/// The background flusher starts at this fraction of the cap.
const DIRTY_BACKGROUND_NUMERATOR: u64 = 3;
const DIRTY_BACKGROUND_DENOMINATOR: u64 = 4;
/// Age at which dirty pages are eligible for writeback, in hundredths of a second.
///
/// An hour, where the kernel default is thirty seconds. Measured 2026-10-08: a DAX install on the
/// large shape wrote 1.97 GB to the workspace mid-run under stock thresholds, while the install's
/// own peak dirty set was about 1.3 GB. The byte cap is what actually bounds the dirty set;
/// raising the age only stops the background flusher from running under a workload that writes a
/// tree and immediately reads it back.
const DIRTY_EXPIRE_CENTISECS: u64 = 360_000;
/// zram is sized to this share of guest RAM.
const ZRAM_DIVISOR: u64 = 4;

const MEMINFO: &str = "/proc/meminfo";
const GIB: u64 = 1024 * 1024 * 1024;

/// The nominal RAM of a machine whose kernel reports `total_bytes` of `MemTotal`.
///
/// `MemTotal` is what the kernel counts after its own reservations, so a sixteen-gigabyte machine
/// reports about 15.996 GiB and never exactly sixteen. The size the machine was built with is that
/// count rounded up to the next whole gigabyte, and it is the size the cap is a quarter of, so the
/// arithmetic matches the shape the contract names rather than the kernel's bookkeeping.
#[must_use]
pub const fn nominal_ram_bytes(total_bytes: u64) -> u64 {
    total_bytes.div_ceil(GIB) * GIB
}

/// The writeback cap for a machine with `total_bytes` of RAM.
///
/// This is `min(RAM / 4, 4 GiB)`, the one rule every shape is sized by: a 16 GiB machine gets
/// 4 GiB and a 4 GiB machine gets 1 GiB, with no per-tier table to keep in step.
#[must_use]
pub const fn dirty_cap(total_bytes: u64) -> u64 {
    let quarter = total_bytes / DIRTY_CAP_DIVISOR;
    if quarter < DIRTY_CAP_CEILING_BYTES {
        quarter
    } else {
        DIRTY_CAP_CEILING_BYTES
    }
}

/// The background threshold for a writeback cap of `cap_bytes`.
#[must_use]
pub const fn dirty_background(cap_bytes: u64) -> u64 {
    cap_bytes / DIRTY_BACKGROUND_DENOMINATOR * DIRTY_BACKGROUND_NUMERATOR
}

/// The zram device size for a machine with `total_bytes` of RAM.
#[must_use]
pub const fn zram_bytes(total_bytes: u64) -> u64 {
    total_bytes / ZRAM_DIVISOR
}

/// Whether the large-only sizing steps apply to a machine with `total_bytes` of RAM.
#[must_use]
pub const fn sizing_applies(total_bytes: u64) -> bool {
    total_bytes > LARGE_FLOOR_BYTES
}

/// One boot's tuning, as the console line that carries it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Report {
    total_bytes: u64,
    cap_bytes: u64,
    zram: Zram,
    lockdown: Option<String>,
    hosts: Option<String>,
}

impl fmt::Display for Report {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hosts = self.hosts.as_deref().unwrap_or("ok");
        if self.cap_bytes == 0 {
            return write!(
                formatter,
                "tune=none ram={} lockdown={} hosts={}",
                self.total_bytes,
                self.lockdown.as_deref().unwrap_or("ok"),
                hosts,
            );
        }
        write!(
            formatter,
            "tune=large ram={} dirty_bytes={} dirty_background_bytes={} dirty_expire_centisecs={} zram={} lockdown={} hosts={}",
            self.total_bytes,
            self.cap_bytes,
            dirty_background(self.cap_bytes),
            DIRTY_EXPIRE_CENTISECS,
            self.zram,
            self.lockdown.as_deref().unwrap_or("ok"),
            hosts,
        )
    }
}

/// Reads `MemTotal` from `/proc/meminfo`, in bytes.
fn total_memory_bytes() -> Option<u64> {
    let text = fs::read_to_string(MEMINFO).ok()?;
    let kib = text
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    Some(kib * 1024)
}

/// Writes one integer sysctl named by `name` below `/proc/sys`, returning a short reason on failure.
pub(crate) fn write_sysctl(name: &str, value: u64) -> Result<(), String> {
    let path = Path::new("/proc/sys").join(name);
    fs::write(&path, value.to_string())
        .map_err(|error| format!("{name}:{}", error.raw_os_error().unwrap_or(0)))
}

/// Applies every tuning step it can and reports what happened.
///
/// Runs before the repair point so a Generation capture taken there carries the result.
#[must_use]
pub fn apply() -> Report {
    let total_bytes = total_memory_bytes().unwrap_or(0);
    // The lockdown step and the hosts line are not sizing decisions: they apply to every machine.
    let lockdown = hardening::lockdown().err();
    let hosts = hardening::ensure_localhost().err();
    if !sizing_applies(total_bytes) {
        return Report {
            total_bytes,
            cap_bytes: 0,
            zram: Zram::NotApplicable,
            lockdown,
            hosts,
        };
    }
    let cap_bytes = dirty_cap(nominal_ram_bytes(total_bytes));
    let zram = match writeback::apply(cap_bytes) {
        Ok(()) => zram::enable(zram_bytes(nominal_ram_bytes(total_bytes))),
        Err(reason) => Zram::Failed(reason),
    };
    Report {
        total_bytes,
        cap_bytes,
        zram,
        lockdown,
        hosts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nominal_ram_rounds_the_kernel_count_up_to_the_built_size() {
        // A sixteen-gigabyte machine reports about 15.996 GiB of `MemTotal`; the built size is 16.
        assert_eq!(nominal_ram_bytes(16 * GIB - 4096 * 1024), 16 * GIB);
        assert_eq!(nominal_ram_bytes(16 * GIB), 16 * GIB);
        assert_eq!(nominal_ram_bytes(1), GIB);
    }

    #[test]
    fn the_writeback_cap_is_a_quarter_of_ram_with_a_four_gigabyte_ceiling() {
        assert_eq!(dirty_cap(GIB), 256 * 1024 * 1024);
        assert_eq!(dirty_cap(16 * GIB), DIRTY_CAP_CEILING_BYTES);
        assert_eq!(dirty_cap(64 * GIB), DIRTY_CAP_CEILING_BYTES);
    }

    #[test]
    fn the_background_threshold_is_three_quarters_of_the_cap() {
        assert_eq!(dirty_background(DIRTY_CAP_CEILING_BYTES), 3 * GIB);
        assert_eq!(dirty_background(dirty_cap(GIB)), 192 * 1024 * 1024);
    }

    #[test]
    fn zram_is_a_quarter_of_ram() {
        assert_eq!(zram_bytes(16 * GIB), 4 * GIB);
        assert_eq!(zram_bytes(GIB), 256 * 1024 * 1024);
    }

    #[test]
    fn the_sizing_steps_are_gated_on_the_version_one_memory_ceiling() {
        // 3 GiB is machine contract v1's `ram-max`; a machine above it can only be v2.
        assert!(!sizing_applies(256 * 1024 * 1024));
        assert!(!sizing_applies(3 * GIB));
        assert!(sizing_applies(3 * GIB + 1));
        assert!(sizing_applies(16 * GIB));
    }

    #[test]
    fn the_report_names_every_step_and_the_gate() {
        let large = Report {
            total_bytes: 16 * GIB,
            cap_bytes: dirty_cap(16 * GIB),
            zram: Zram::Enabled(4 * GIB),
            lockdown: None,
            hosts: None,
        }
        .to_string();
        assert!(large.contains("dirty_bytes=4294967296"), "{large}");
        assert!(large.contains("dirty_background_bytes=3221225472"), "{large}");
        assert!(large.contains("dirty_expire_centisecs=360000"), "{large}");
        assert!(large.contains("zram=enabled:4294967296"), "{large}");
        assert!(large.contains("lockdown=ok"), "{large}");
        assert!(large.contains("hosts=ok"), "{large}");

        let small = Report {
            total_bytes: 256 * 1024 * 1024,
            cap_bytes: 0,
            zram: Zram::NotApplicable,
            lockdown: None,
            hosts: None,
        }
        .to_string();
        assert!(small.contains("tune=none"), "{small}");
        assert!(!small.contains("dirty_bytes"), "{small}");
    }
}
