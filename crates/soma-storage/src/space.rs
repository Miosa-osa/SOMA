//! How much room one filesystem has, for a capacity report.

#![allow(unsafe_code)]

use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;

/// The size of the filesystem holding a path, and what an unprivileged writer may still use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Space {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

/// Reads the filesystem that holds `path`.
///
/// # Errors
///
/// Returns the `statvfs` failure, or an error for a path holding a NUL byte.
pub fn of(path: &Path) -> io::Result<Space> {
    let name = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::other("a path holds a NUL byte"))?;
    // SAFETY: `statvfs` is plain-old-data, so the all-zero value is a valid instance for the
    // kernel to overwrite, and `name` is a NUL-terminated string that outlives the call.
    let stats = unsafe {
        let mut stats: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(name.as_ptr(), &raw mut stats) != 0 {
            return Err(io::Error::last_os_error());
        }
        stats
    };
    // The field widths differ between targets, so the widening is explicit where it is one.
    #[allow(clippy::useless_conversion)]
    let (unit, blocks, available) = (
        u64::from(stats.f_frsize),
        u64::from(stats.f_blocks),
        u64::from(stats.f_bavail),
    );
    Ok(Space {
        total_bytes: unit.saturating_mul(blocks),
        available_bytes: unit.saturating_mul(available),
    })
}
