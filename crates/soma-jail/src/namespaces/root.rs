//! Entering the empty read-only root with `pivot_root`.
//!
//! `chroot` is never used because it leaves the old root reachable through an open directory
//! or a `..` walk; `pivot_root` followed by a detached unmount removes the old root from the
//! mount namespace entirely.

#![allow(unsafe_code)]

use std::{
    ffi::CStr,
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
};

/// Which `pivot_root` step failed in the child.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootStep {
    OpenOldRoot,
    MakeRootPrivate,
    MountEmptyRoot,
    EnterNewRoot,
    PivotRoot,
    DetachOldRoot,
    ChdirRoot,
}

fn last_errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn step(step: RootStep, result: libc::c_int) -> Result<(), (RootStep, i32)> {
    if result == 0 {
        Ok(())
    } else {
        Err((step, last_errno()))
    }
}

/// Mounts a read-only tmpfs on `path`, pivots into it, and detaches the old root.
///
/// Allocation-free and safe to call in a freshly cloned child.
///
/// # Errors
///
/// Returns the failing [`RootStep`] with its errno.
pub(crate) fn enter_empty_root(path: &CStr) -> Result<(), (RootStep, i32)> {
    // SAFETY: every pointer below is a valid NUL-terminated literal or `path`, which outlives
    // the calls; the kernel copies what it needs before returning.
    unsafe {
        let old_root = libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        );
        if old_root < 0 {
            return Err((RootStep::OpenOldRoot, last_errno()));
        }
        let old_root = OwnedFd::from_raw_fd(old_root);
        step(
            RootStep::MakeRootPrivate,
            libc::mount(
                std::ptr::null(),
                c"/".as_ptr(),
                std::ptr::null(),
                libc::MS_REC | libc::MS_PRIVATE,
                std::ptr::null(),
            ),
        )?;
        step(
            RootStep::MountEmptyRoot,
            libc::mount(
                c"tmpfs".as_ptr(),
                path.as_ptr(),
                c"tmpfs".as_ptr(),
                libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
                c"size=4k,mode=0555".as_ptr().cast(),
            ),
        )?;
        step(RootStep::EnterNewRoot, libc::chdir(path.as_ptr()))?;
        let pivoted = libc::syscall(libc::SYS_pivot_root, c".".as_ptr(), c".".as_ptr());
        step(
            RootStep::PivotRoot,
            libc::c_int::try_from(pivoted).unwrap_or(-1),
        )?;
        step(RootStep::DetachOldRoot, libc::fchdir(old_root.as_raw_fd()))?;
        step(
            RootStep::DetachOldRoot,
            libc::umount2(c".".as_ptr(), libc::MNT_DETACH),
        )?;
        step(RootStep::ChdirRoot, libc::chdir(c"/".as_ptr()))?;
        drop(old_root);
    }
    Ok(())
}
