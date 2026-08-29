//! Reading, parsing, and encoding cgroup v2 interface files.

use std::{ffi::CString, fs, io, os::unix::ffi::OsStrExt, path::Path};

use super::CgroupError;
use crate::spec::IoMax;

/// `CGROUP2_SUPER_MAGIC` from `linux/magic.h`: the bytes `cgrp`.
const CGROUP2_MAGIC: i64 = 0x6367_7270;

pub(super) fn errno_of(error: &io::Error) -> i32 {
    error.raw_os_error().unwrap_or(0)
}

pub(super) fn readback(file: &'static str, expected: &str, found: &str) -> CgroupError {
    CgroupError::Readback {
        file,
        expected: expected.to_owned(),
        found: found.to_owned(),
    }
}

/// Parses a cgroup limit, where `max` means unbounded.
pub(super) fn parse_max(file: &'static str, value: &str) -> Result<u64, CgroupError> {
    if value == "max" {
        return Ok(u64::MAX);
    }
    value
        .parse()
        .map_err(|_| readback(file, "integer or max", value))
}

/// Reads one interface file and trims it.
pub(super) fn read(root: &Path, file: &'static str) -> Result<String, CgroupError> {
    fs::read_to_string(root.join(file))
        .map(|value| value.trim().to_owned())
        .map_err(|error| CgroupError::Read {
            file,
            errno: errno_of(&error),
        })
}

/// The `io.max` line for one device; a zero dimension is written as `max`.
pub(super) fn io_max_line(io: IoMax) -> String {
    let bound = |value: u64| {
        if value == 0 {
            "max".to_owned()
        } else {
            value.to_string()
        }
    };
    format!(
        "{}:{} rbps={} wbps={} riops={} wiops={}",
        io.major,
        io.minor,
        bound(io.read_bytes_per_second),
        bound(io.write_bytes_per_second),
        bound(io.read_iops),
        bound(io.write_iops)
    )
}

/// Fails unless `root` sits on a cgroup2 filesystem.
#[allow(unsafe_code)]
pub(super) fn require_cgroup2(root: &Path) -> Result<(), CgroupError> {
    let path = CString::new(root.as_os_str().as_bytes()).map_err(|_| CgroupError::NotCgroup2)?;
    // SAFETY: `statfs` is zeroed storage the kernel fills on success.
    let mut statfs: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is a valid NUL-terminated string and `statfs` is valid writable storage.
    let result = unsafe { libc::statfs(path.as_ptr(), &raw mut statfs) };
    #[allow(clippy::useless_conversion, clippy::unnecessary_cast)]
    let kind = i64::try_from(statfs.f_type).unwrap_or(-1);
    if result != 0 || kind != CGROUP2_MAGIC {
        return Err(CgroupError::NotCgroup2);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_max_uses_max_for_unbounded_dimensions() {
        let line = io_max_line(IoMax {
            major: 8,
            minor: 16,
            read_bytes_per_second: 1_000_000,
            write_bytes_per_second: 0,
            read_iops: 0,
            write_iops: 500,
        });
        assert_eq!(line, "8:16 rbps=1000000 wbps=max riops=max wiops=500");
    }

    /// Checks the real cgroup2 mount when `/proc/self/mounts` proves one exists at the
    /// conventional path; a host without one cannot make this claim either way.
    #[test]
    fn the_cgroup2_mount_is_recognized() {
        let mounts = fs::read_to_string("/proc/self/mounts").unwrap_or_default();
        let mounted = mounts.lines().any(|line| {
            let mut fields = line.split_whitespace();
            fields.next();
            fields.next() == Some("/sys/fs/cgroup") && fields.next() == Some("cgroup2")
        });
        if mounted {
            assert_eq!(require_cgroup2(Path::new("/sys/fs/cgroup")), Ok(()));
        }
    }

    #[test]
    fn a_plain_directory_is_not_cgroup2() {
        let root = std::env::temp_dir();
        assert_eq!(require_cgroup2(&root), Err(CgroupError::NotCgroup2));
        assert_eq!(parse_max("memory.max", "max"), Ok(u64::MAX));
        assert!(parse_max("memory.max", "lots").is_err());
    }
}
