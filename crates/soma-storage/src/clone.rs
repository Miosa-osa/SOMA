//! Private head creation with `FICLONE` under a capability directory descriptor.
//!
//! The caller passes an open read-only template and an open directory; it receives an open
//! read-write descriptor for the new head and never a path, and chooses whether the head is
//! published durably or is ephemeral and therefore never synced.
//! Any failure after the destination exists unlinks it before the error is returned.

#![allow(unsafe_code)]

use std::ffi::CString;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

use crate::fiemap::{self, ExtentSummary};
use crate::head::HeadName;

mod error;

pub use error::CloneError;

/// One created and verified head.
#[derive(Debug)]
pub struct ClonedHead {
    fd: OwnedFd,
    apparent_bytes: u64,
    extents: ExtentSummary,
}

impl ClonedHead {
    /// Transfers the open descriptor to the caller.
    #[must_use]
    pub fn into_fd(self) -> OwnedFd {
        self.fd
    }

    /// Apparent size, equal to the template size.
    #[must_use]
    pub fn apparent_bytes(&self) -> u64 {
        self.apparent_bytes
    }

    /// Extent summary observed after the clone; every extent was shared.
    #[must_use]
    pub fn extents(&self) -> ExtentSummary {
        self.extents
    }
}

impl AsFd for ClonedHead {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

/// Wall-clock cost of each clone phase, retained for the benchmark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClonePhases {
    /// Exclusive creation of the destination.
    pub create: Duration,
    /// The `FICLONE` call.
    pub clone: Duration,
    /// `fsync` of the destination file, zero for an ephemeral head.
    pub file_sync: Duration,
    /// `fsync` of the directory that publishes the name, zero for an ephemeral head.
    pub dir_sync: Duration,
    /// Size and extent-sharing verification.
    pub verify: Duration,
}

impl ClonePhases {
    /// Sum of every phase.
    #[must_use]
    pub fn total(&self) -> Duration {
        self.create + self.clone + self.file_sync + self.dir_sync + self.verify
    }
}

/// Whether a head's name and extent map are made durable before it is handed over.
///
/// A head that is unlinked the instant it is created and read only through the descriptor its
/// creator keeps has nothing to survive a crash for: the two `fsync` calls that publish it push
/// the filesystem log for a file that must not outlive its sandbox. Extent sharing is still
/// proved either way, because the `FIEMAP` verification asks the kernel to flush the inode
/// itself before it maps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Durability {
    /// `fsync` the head and then the directory that names it.
    Persisted,
    /// Skip both syncs; the head is ephemeral and is not published to anyone.
    Ephemeral,
}

/// Creates the head `name` under `dir` as a reflink clone of `template`.
///
/// # Errors
///
/// Returns the first failing step; a created destination is unlinked before returning.
pub fn clone_head(
    template: BorrowedFd<'_>,
    dir: BorrowedFd<'_>,
    name: &HeadName,
    durability: Durability,
) -> Result<ClonedHead, CloneError> {
    clone_head_timed(template, dir, name, durability).map(|(head, _)| head)
}

/// [`clone_head`] with the duration of every phase.
///
/// # Errors
///
/// Same as [`clone_head`].
pub fn clone_head_timed(
    template: BorrowedFd<'_>,
    dir: BorrowedFd<'_>,
    name: &HeadName,
    durability: Durability,
) -> Result<(ClonedHead, ClonePhases), CloneError> {
    let c_name =
        CString::new(name.as_str()).map_err(|error| CloneError::Create(io::Error::other(error)))?;
    let mut phases = ClonePhases::default();
    let started = Instant::now();
    let fd = create_exclusive(dir, &c_name)?;
    phases.create = started.elapsed();
    match finish(template, dir, fd.as_fd(), durability, &mut phases) {
        Ok((apparent_bytes, extents)) => Ok((
            ClonedHead {
                fd,
                apparent_bytes,
                extents,
            },
            phases,
        )),
        Err(error) => {
            unlink_quietly(dir, &c_name);
            Err(error)
        }
    }
}

fn finish(
    template: BorrowedFd<'_>,
    dir: BorrowedFd<'_>,
    fd: BorrowedFd<'_>,
    durability: Durability,
    phases: &mut ClonePhases,
) -> Result<(u64, ExtentSummary), CloneError> {
    let started = Instant::now();
    // SAFETY: `FICLONE` takes the source descriptor as its integer argument; both descriptors
    // are live for the duration of the call.
    let rc = unsafe { libc::ioctl(fd.as_raw_fd(), libc::FICLONE, template.as_raw_fd()) };
    if rc != 0 {
        return Err(error::classify_clone_error(io::Error::last_os_error()));
    }
    phases.clone = started.elapsed();

    if durability == Durability::Persisted {
        let started = Instant::now();
        fsync(fd).map_err(CloneError::FileSync)?;
        phases.file_sync = started.elapsed();

        let started = Instant::now();
        fsync(dir).map_err(CloneError::DirSync)?;
        phases.dir_sync = started.elapsed();
    }

    let started = Instant::now();
    let expected = file_size(template).map_err(CloneError::Verify)?;
    let actual = file_size(fd).map_err(CloneError::Verify)?;
    if expected != actual {
        return Err(CloneError::SizeMismatch { expected, actual });
    }
    let extents = fiemap::summarize(fd).map_err(CloneError::Verify)?;
    if !extents.all_shared() {
        return Err(CloneError::ExtentsNotShared {
            extents: extents.extents,
            shared: extents.shared_extents,
        });
    }
    phases.verify = started.elapsed();
    Ok((actual, extents))
}

fn create_exclusive(dir: BorrowedFd<'_>, name: &CString) -> Result<OwnedFd, CloneError> {
    let flags = libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW;
    // SAFETY: `name` is NUL-terminated and outlives the call, `dir` is a live directory
    // descriptor, and the mode is the only extra argument the `O_CREAT` form of `openat`
    // reads; a non-negative result is a descriptor that nothing else owns.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags, 0o600 as libc::c_uint) };
    if fd < 0 {
        let error = io::Error::last_os_error();
        return Err(error::classify_create_error(error));
    }
    // SAFETY: `fd` was just returned by `openat` and is owned by no one else.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Unlinks `name` under `dir`; failures are ignored because the caller is already failing.
pub(crate) fn unlink_quietly(dir: BorrowedFd<'_>, name: &CString) {
    // SAFETY: `name` is NUL-terminated and outlives the call and `dir` is a live descriptor.
    let _ = unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) };
}

/// `fsync` of any descriptor.
///
/// # Errors
///
/// Propagates the kernel failure.
pub(crate) fn fsync(fd: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: `fd` is a live descriptor for the duration of the call.
    if unsafe { libc::fsync(fd.as_raw_fd()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Apparent size of an open file.
///
/// # Errors
///
/// Propagates the `fstat` failure or a negative size.
pub(crate) fn file_size(fd: BorrowedFd<'_>) -> io::Result<u64> {
    // SAFETY: `stat` is plain-old-data, so the all-zero value is valid for the kernel to
    // overwrite, and `fd` is a live descriptor for the duration of the call.
    let stats = unsafe {
        let mut stats: libc::stat = std::mem::zeroed();
        if libc::fstat(fd.as_raw_fd(), &raw mut stats) != 0 {
            return Err(io::Error::last_os_error());
        }
        stats
    };
    u64::try_from(stats.st_size).map_err(|_| io::Error::other("negative file size"))
}
