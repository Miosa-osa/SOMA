//! Narrow typed wrappers over the Linux mount family used by early init and repair.

#![allow(unsafe_code)]

use std::ffi::CString;
use std::io;

/// A raw Linux errno from a failed mount-family call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Errno(pub i32);

impl Errno {
    fn last() -> Self {
        Self(io::Error::last_os_error().raw_os_error().unwrap_or(0))
    }
}

/// Mounts `source` at `target` with a fixed filesystem type, flags, and option string.
///
/// # Errors
///
/// Returns the kernel errno, or `EINVAL` for an argument containing NUL.
pub fn mount(
    source: &str,
    target: &str,
    fstype: &str,
    flags: libc::c_ulong,
    data: &str,
) -> Result<(), Errno> {
    let source = c_string(source)?;
    let target = c_string(target)?;
    let fstype = c_string(fstype)?;
    let data = c_string(data)?;
    // SAFETY: every pointer refers to a NUL-terminated `CString` that outlives the call, the
    // data pointer is a plain option string, and the flags are fixed kernel constants.
    let result = unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            fstype.as_ptr(),
            flags,
            data.as_ptr().cast(),
        )
    };
    check(result)
}

/// Moves an existing mount from `from` to `to`.
///
/// # Errors
///
/// Returns the kernel errno.
pub fn move_mount(from: &str, to: &str) -> Result<(), Errno> {
    let from = c_string(from)?;
    let to = c_string(to)?;
    // SAFETY: both pointers are valid NUL-terminated strings; `MS_MOVE` ignores the null
    // filesystem type and data pointers.
    let result = unsafe {
        libc::mount(
            from.as_ptr(),
            to.as_ptr(),
            std::ptr::null(),
            libc::MS_MOVE,
            std::ptr::null(),
        )
    };
    check(result)
}

/// Changes the root directory of this process to `path`.
///
/// The initial ramfs root has no parent mount, so `pivot_root` cannot leave it; the composed
/// root is therefore moved over `/` and entered with `chroot` exactly as `switch_root` does.
///
/// # Errors
///
/// Returns the kernel errno.
pub fn chroot(path: &str) -> Result<(), Errno> {
    let path = c_string(path)?;
    // SAFETY: the pointer is a valid NUL-terminated string for the duration of the call.
    check(unsafe { libc::chroot(path.as_ptr()) })
}

fn c_string(value: &str) -> Result<CString, Errno> {
    CString::new(value).map_err(|_| Errno(libc::EINVAL))
}

fn check(result: libc::c_int) -> Result<(), Errno> {
    if result == 0 {
        Ok(())
    } else {
        Err(Errno::last())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nul_bytes_never_reach_the_kernel() {
        assert_eq!(
            mount("a\0b", "/nonexistent", "tmpfs", 0, ""),
            Err(Errno(libc::EINVAL))
        );
        assert_eq!(chroot("x\0"), Err(Errno(libc::EINVAL)));
    }

    #[test]
    fn an_unprivileged_mount_fails_with_a_kernel_errno() {
        let error = mount("tmpfs", "/proc/self/nonexistent", "tmpfs", 0, "")
            .expect_err("mount must fail without privilege or target");
        assert!(matches!(error, Errno(libc::EPERM | libc::ENOENT)));
    }
}
