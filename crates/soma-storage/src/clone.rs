//! Private head creation with `FICLONE` under a capability directory descriptor.
//!
//! The caller passes an open read-only template and an open directory; it receives an open
//! read-write descriptor for the new head and never a path.
//! Any failure after the destination exists unlinks it before the error is returned.

#![allow(unsafe_code)]

use std::ffi::CString;
use std::fmt;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

use crate::fiemap::{self, ExtentSummary};
use crate::head::HeadName;

/// One created, synced, and verified head.
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
    /// `fsync` of the destination file.
    pub file_sync: Duration,
    /// `fsync` of the directory that publishes the name.
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

/// Why a head could not be created.
#[derive(Debug)]
pub enum CloneError {
    /// The name already exists in the directory.
    AlreadyExists,
    /// Exclusive creation failed for another reason.
    Create(io::Error),
    /// The filesystem has no space for the clone metadata.
    NoSpace,
    /// The kernel refused `FICLONE`; the mount is not a certified reflink profile.
    ReflinkUnsupported,
    /// Template and destination live on different filesystems.
    CrossDevice,
    /// `FICLONE` failed for another reason.
    Clone(io::Error),
    /// `fsync` of the destination failed.
    FileSync(io::Error),
    /// `fsync` of the directory failed.
    DirSync(io::Error),
    /// Size or extent verification could not run.
    Verify(io::Error),
    /// The destination size differs from the template.
    SizeMismatch {
        /// Template size.
        expected: u64,
        /// Destination size.
        actual: u64,
    },
    /// At least one destination extent is not shared with the template.
    ExtentsNotShared {
        /// Extents reported.
        extents: u64,
        /// Extents flagged shared.
        shared: u64,
    },
}

impl fmt::Display for CloneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists => f.write_str("head name already exists"),
            Self::Create(error) => write!(f, "head creation failed: {error}"),
            Self::NoSpace => f.write_str("filesystem has no space for the head"),
            Self::ReflinkUnsupported => f.write_str("filesystem refused FICLONE"),
            Self::CrossDevice => f.write_str("template and directory are on different filesystems"),
            Self::Clone(error) => write!(f, "FICLONE failed: {error}"),
            Self::FileSync(error) => write!(f, "head fsync failed: {error}"),
            Self::DirSync(error) => write!(f, "directory fsync failed: {error}"),
            Self::Verify(error) => write!(f, "head verification failed: {error}"),
            Self::SizeMismatch { expected, actual } => {
                write!(f, "head has {actual} bytes, template has {expected}")
            }
            Self::ExtentsNotShared { extents, shared } => {
                write!(f, "only {shared} of {extents} head extents are shared")
            }
        }
    }
}

impl std::error::Error for CloneError {}

/// Creates the head `name` under `dir` as a reflink clone of `template`.
///
/// # Errors
///
/// Returns the first failing step; a created destination is unlinked before returning.
pub fn clone_head(
    template: BorrowedFd<'_>,
    dir: BorrowedFd<'_>,
    name: &HeadName,
) -> Result<ClonedHead, CloneError> {
    clone_head_timed(template, dir, name).map(|(head, _)| head)
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
) -> Result<(ClonedHead, ClonePhases), CloneError> {
    let c_name =
        CString::new(name.as_str()).map_err(|error| CloneError::Create(io::Error::other(error)))?;
    let mut phases = ClonePhases::default();
    let started = Instant::now();
    let fd = create_exclusive(dir, &c_name)?;
    phases.create = started.elapsed();
    match finish(template, dir, fd.as_fd(), &mut phases) {
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
    phases: &mut ClonePhases,
) -> Result<(u64, ExtentSummary), CloneError> {
    let started = Instant::now();
    // SAFETY: `FICLONE` takes the source descriptor as its integer argument; both descriptors
    // are live for the duration of the call.
    let rc = unsafe { libc::ioctl(fd.as_raw_fd(), libc::FICLONE, template.as_raw_fd()) };
    if rc != 0 {
        return Err(classify_clone_error(io::Error::last_os_error()));
    }
    phases.clone = started.elapsed();

    let started = Instant::now();
    fsync(fd).map_err(CloneError::FileSync)?;
    phases.file_sync = started.elapsed();

    let started = Instant::now();
    fsync(dir).map_err(CloneError::DirSync)?;
    phases.dir_sync = started.elapsed();

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

fn classify_clone_error(error: io::Error) -> CloneError {
    match error.raw_os_error() {
        Some(libc::ENOSPC | libc::EDQUOT) => CloneError::NoSpace,
        Some(libc::EOPNOTSUPP | libc::ENOTTY | libc::EINVAL) => CloneError::ReflinkUnsupported,
        Some(libc::EXDEV) => CloneError::CrossDevice,
        _ => CloneError::Clone(error),
    }
}

fn create_exclusive(dir: BorrowedFd<'_>, name: &CString) -> Result<OwnedFd, CloneError> {
    let flags = libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW;
    // SAFETY: `name` is NUL-terminated and outlives the call, `dir` is a live directory
    // descriptor, and the mode is the only extra argument the `O_CREAT` form of `openat`
    // reads; a non-negative result is a descriptor that nothing else owns.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags, 0o600 as libc::c_uint) };
    if fd < 0 {
        let error = io::Error::last_os_error();
        return Err(match error.raw_os_error() {
            Some(libc::EEXIST) => CloneError::AlreadyExists,
            Some(libc::ENOSPC | libc::EDQUOT) => CloneError::NoSpace,
            _ => CloneError::Create(error),
        });
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
