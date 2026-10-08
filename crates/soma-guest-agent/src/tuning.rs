//! The guest tuning the large shape depends on, applied before the capture point.
//!
//! Machine contract v2 admits eight processors and sixteen gigabytes, which is the shape a build
//! workload runs in: an install writes a hundred thousand small files and a typecheck reads them
//! all back. Three settings turn that from "correct" into "fast", and all three are guest state,
//! so they are applied at boot and captured with the machine. Every restored Instance therefore
//! starts already tuned instead of running a tuning script of its own.
//!
//! Nothing here is a security boundary and nothing here fails a boot. A guest whose kernel was
//! built without swap, or whose image carries no `mkswap`, must still start: tuning is a
//! performance claim, and a machine that refuses to run because it could not make itself faster
//! has traded a working sandbox for a measurement. So every step reports what it did, and the
//! report is one console line the boot evidence carries.

use std::{
    fmt, fs,
    path::{Path, PathBuf},
    process::Command,
};

const MEMINFO: &str = "/proc/meminfo";
const SYSCTL_DIR: &str = "/proc/sys/vm";
/// Where the kernel exposes a zram device when it has one.
const ZRAM_DIR: &str = "/sys/block/zram0";
const ZRAM_DEVICE: &str = "/dev/zram0";
/// The writeback cap never exceeds this, however much RAM the machine has.
///
/// A ratio is a percentage of available memory, which shrinks exactly when the machine is tight,
/// and dirty pages cannot be reclaimed until they are written. A byte cap bounds the worst case
/// on every machine size, which is why this one is a byte count and the ratios below are zero.
const DIRTY_CAP_CEILING_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Guest RAM is divided by this to size the writeback cap.
const DIRTY_CAP_DIVISOR: u64 = 4;
/// The background flusher starts at three quarters of the cap.
const DIRTY_BACKGROUND_NUMERATOR: u64 = 3;
const DIRTY_BACKGROUND_DENOMINATOR: u64 = 4;
/// Age at which dirty pages are eligible for writeback, in hundredths of a second.
///
/// An hour, where the default is thirty seconds: a workload that writes a tree and reads it back
/// does not want the flusher running under it, and the byte cap above is what actually bounds the
/// dirty set.
const DIRTY_EXPIRE_CENTISECS: u64 = 360_000;
/// zram is sized to this share of guest RAM.
const ZRAM_DIVISOR: u64 = 4;

/// The writeback cap for a machine with `total_bytes` of RAM.
///
/// This is `min(RAM / 4, 4 GiB)`, the one rule the plan sizes every shape by: a 16 GiB machine
/// gets 4 GiB and a 4 GiB machine gets 1 GiB, with no per-tier table to keep in step.
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

/// What the zram step found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Zram {
    /// The kernel has no zram device at all, which is a kernel configuration and not a failure.
    NoKernelDevice,
    /// The kernel has one, and the image has no tool to put a swap area on it.
    NoToolsAvailable,
    /// The device was sized, formatted, and enabled at this size.
    Enabled(u64),
    /// The kernel has one and the step failed; the string is what failed.
    Failed(String),
}

/// One boot's tuning, as the console line that carries it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Report {
    total_bytes: u64,
    cap_bytes: u64,
    zram: Zram,
}

impl fmt::Display for Report {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "tuned dirty_bytes={} dirty_background_bytes={} zram={}",
            self.cap_bytes,
            dirty_background(self.cap_bytes),
            self.zram
        )
    }
}

impl fmt::Display for Zram {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoKernelDevice => formatter.write_str("no-kernel-device"),
            Self::NoToolsAvailable => formatter.write_str("no-mkswap"),
            Self::Enabled(bytes) => write!(formatter, "enabled:{bytes}"),
            Self::Failed(reason) => write!(formatter, "failed:{reason}"),
        }
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

/// Writes one integer sysctl, returning the first failure as a short reason.
fn write_sysctl(name: &str, value: u64) -> Result<(), String> {
    let path = Path::new(SYSCTL_DIR).join(name);
    fs::write(&path, value.to_string()).map_err(|error| format!("{name}:{error}"))
}

/// Applies every tuning step it can and reports what happened.
#[must_use]
pub fn apply() -> Report {
    let Some(total_bytes) = total_memory_bytes() else {
        return Report {
            total_bytes: 0,
            cap_bytes: 0,
            zram: Zram::Failed("unreadable-meminfo".to_owned()),
        };
    };
    let cap_bytes = dirty_cap(total_bytes);
    let background = dirty_background(cap_bytes);
    // The ratios are forced to zero first, because the kernel ignores `dirty_bytes` while a
    // non-zero `dirty_ratio` is set, and a failure to zero one is reported rather than ignored.
    let mut failures: Vec<String> = Vec::new();
    for (name, value) in [
        ("dirty_ratio", 0),
        ("dirty_background_ratio", 0),
        ("dirty_bytes", cap_bytes),
        ("dirty_background_bytes", background),
        ("dirty_expire_centisecs", DIRTY_EXPIRE_CENTISECS),
    ] {
        if let Err(reason) = write_sysctl(name, value) {
            failures.push(reason);
        }
    }
    let zram = if failures.is_empty() {
        enable_zram(total_bytes)
    } else {
        Zram::Failed(failures.join(","))
    };
    Report {
        total_bytes,
        cap_bytes,
        zram,
    }
}

/// Sizes, formats, and enables a zram swap device when the kernel and the image allow one.
fn enable_zram(total_bytes: u64) -> Zram {
    let directory = PathBuf::from(ZRAM_DIR);
    if !directory.is_dir() {
        return Zram::NoKernelDevice;
    }
    let size = zram_bytes(total_bytes);
    if let Err(error) = fs::write(directory.join("disksize"), size.to_string()) {
        return Zram::Failed(format!("disksize:{error}"));
    }
    // The device is used as swap, and `swapon` requires a swap area written by `mkswap`. Both are
    // ordinary image tools rather than kernel interfaces, so an image without them reports that
    // instead of failing the boot.
    for program in ["/sbin/mkswap", "/usr/sbin/mkswap"] {
        if !Path::new(program).exists() {
            continue;
        }
        if let Err(error) = run(program, ZRAM_DEVICE) {
            return Zram::Failed(error);
        }
        for swapon in ["/sbin/swapon", "/usr/sbin/swapon"] {
            if !Path::new(swapon).exists() {
                continue;
            }
            return match run(swapon, ZRAM_DEVICE) {
                Ok(()) => Zram::Enabled(size),
                Err(error) => Zram::Failed(error),
            };
        }
        return Zram::NoToolsAvailable;
    }
    Zram::NoToolsAvailable
}

/// Runs one image tool on one device, returning a short reason on failure.
fn run(program: &str, argument: &str) -> Result<(), String> {
    match Command::new(program).arg(argument).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("{program}:{status}")),
        Err(error) => Err(format!("{program}:{error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn the_writeback_cap_is_a_quarter_of_ram_with_a_four_gigabyte_ceiling() {
        assert_eq!(dirty_cap(GIB), 256 * 1024 * 1024);
        assert_eq!(dirty_cap(16 * GIB), DIRTY_CAP_CEILING_BYTES);
        assert_eq!(dirty_cap(64 * GIB), DIRTY_CAP_CEILING_BYTES);
    }

    #[test]
    fn the_background_threshold_is_three_quarters_of_the_cap() {
        let cap = dirty_cap(16 * GIB);
        assert_eq!(dirty_background(cap), 3 * GIB);
        assert_eq!(dirty_background(dirty_cap(GIB)), 192 * 1024 * 1024);
    }

    #[test]
    fn zram_is_a_quarter_of_ram() {
        assert_eq!(zram_bytes(16 * GIB), 4 * GIB);
        assert_eq!(zram_bytes(GIB), 256 * 1024 * 1024);
    }

    #[test]
    fn the_report_names_every_step() {
        let report = Report {
            total_bytes: 16 * GIB,
            cap_bytes: dirty_cap(16 * GIB),
            zram: Zram::Enabled(4 * GIB),
        };
        let line = report.to_string();
        assert!(line.contains("dirty_bytes=4294967296"), "{line}");
        assert!(line.contains("dirty_background_bytes=3221225472"), "{line}");
        assert!(line.contains("zram=enabled:4294967296"), "{line}");
        assert!(
            Report {
                total_bytes: 0,
                cap_bytes: 0,
                zram: Zram::NoKernelDevice
            }
            .to_string()
            .contains("zram=no-kernel-device")
        );
    }
}
