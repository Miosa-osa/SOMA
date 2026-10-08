//! D3: a zram swap device sized to a quarter of RAM.
//!
//! A build workload's peak is a spike, not a plateau: a compiler or a bundler allocates a working
//! set it releases a moment later. A compressed swap device in RAM turns that spike into headroom
//! at the cost of some CPU instead of an out-of-memory kill, and a quarter of RAM is small enough
//! that it hides a spike rather than a leak.
//!
//! The kernel must have `zram` built in and the image must carry `mkswap` and `swapon`. This is a
//! best-effort step: a guest on a kernel without the device, or an image without the tools,
//! reports what it found and starts anyway.

use std::{fmt, fs, path::Path, process::Command};

/// Where the kernel exposes the first zram device when it has one.
pub const ZRAM_SYSFS: &str = "/sys/block/zram0";
/// The device node the swap area is written to.
pub const ZRAM_DEVICE: &str = "/dev/zram0";
/// Image locations of the two tools that make and enable a swap area.
const MKSWAP: [&str; 2] = ["/sbin/mkswap", "/usr/sbin/mkswap"];
const SWAPON: [&str; 2] = ["/sbin/swapon", "/usr/sbin/swapon"];

/// What the zram step found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Zram {
    /// The large-only sizing steps do not apply to this machine.
    NotApplicable,
    /// The kernel has no zram device at all, which is a kernel configuration and not a failure.
    NoKernelDevice,
    /// The kernel has one, and the image has no tool to put a swap area on it.
    NoToolsAvailable,
    /// The device was sized, formatted, and enabled at this size.
    Enabled(u64),
    /// A step failed; the string is what failed.
    Failed(String),
}

impl fmt::Display for Zram {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotApplicable => formatter.write_str("n/a"),
            Self::NoKernelDevice => formatter.write_str("no-kernel-device"),
            Self::NoToolsAvailable => formatter.write_str("no-mkswap"),
            Self::Enabled(bytes) => write!(formatter, "enabled:{bytes}"),
            Self::Failed(reason) => write!(formatter, "failed:{reason}"),
        }
    }
}

/// Sizes, formats, and enables a zram swap device of `bytes` when the kernel and image allow one.
pub(super) fn enable(bytes: u64) -> Zram {
    if !Path::new(ZRAM_SYSFS).is_dir() {
        return Zram::NoKernelDevice;
    }
    if let Err(error) = fs::write(Path::new(ZRAM_SYSFS).join("disksize"), bytes.to_string()) {
        return Zram::Failed(format!("disksize:{}", error.raw_os_error().unwrap_or(0)));
    }
    // `swapon` requires a swap area written by `mkswap`. Both are ordinary image tools rather than
    // kernel interfaces, so an image without them reports that instead of failing the boot.
    let Some(mkswap) = first_present(&MKSWAP) else {
        return Zram::NoToolsAvailable;
    };
    if let Err(reason) = run(mkswap, ZRAM_DEVICE) {
        return Zram::Failed(reason);
    }
    let Some(swapon) = first_present(&SWAPON) else {
        return Zram::NoToolsAvailable;
    };
    match run(swapon, ZRAM_DEVICE) {
        Ok(()) => Zram::Enabled(bytes),
        Err(reason) => Zram::Failed(reason),
    }
}

/// The first of `candidates` that exists on the image.
fn first_present(candidates: &[&'static str]) -> Option<&'static str> {
    candidates
        .iter()
        .copied()
        .find(|path| Path::new(path).exists())
}

/// Runs one image tool on one device, returning a short reason on failure.
fn run(program: &str, argument: &str) -> Result<(), String> {
    match Command::new(program).arg(argument).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("{program}:{status}")),
        Err(error) => Err(format!("{program}:{}", error.raw_os_error().unwrap_or(0))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_names_each_outcome() {
        assert_eq!(Zram::NotApplicable.to_string(), "n/a");
        assert_eq!(Zram::NoKernelDevice.to_string(), "no-kernel-device");
        assert_eq!(Zram::NoToolsAvailable.to_string(), "no-mkswap");
        assert_eq!(Zram::Enabled(4096).to_string(), "enabled:4096");
        assert_eq!(Zram::Failed("x".to_owned()).to_string(), "failed:x");
    }

    #[test]
    fn a_tool_list_is_searched_in_order_and_a_missing_one_is_not_found() {
        assert_eq!(
            first_present(&["/definitely/not/here-a", "/definitely/not/here-b"]),
            None
        );
        assert_eq!(first_present(&["/bin/sh"]), Some("/bin/sh"));
    }
}
