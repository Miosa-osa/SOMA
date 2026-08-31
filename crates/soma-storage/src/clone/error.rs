//! Why a private head could not be created, and how a kernel errno becomes one of those reasons.

use std::fmt;
use std::io;

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

/// The reason behind a failed `FICLONE`.
///
/// Only the absence of the capability is reported as such: every other errno is a real failure
/// and must not be mistaken for a filesystem that cannot reflink.
pub(super) fn classify_clone_error(error: io::Error) -> CloneError {
    match error.raw_os_error() {
        Some(libc::ENOSPC | libc::EDQUOT) => CloneError::NoSpace,
        Some(libc::EOPNOTSUPP | libc::ENOTTY | libc::EINVAL) => CloneError::ReflinkUnsupported,
        Some(libc::EXDEV) => CloneError::CrossDevice,
        _ => CloneError::Clone(error),
    }
}

/// The reason behind a failed exclusive creation.
pub(super) fn classify_create_error(error: io::Error) -> CloneError {
    match error.raw_os_error() {
        Some(libc::EEXIST) => CloneError::AlreadyExists,
        Some(libc::ENOSPC | libc::EDQUOT) => CloneError::NoSpace,
        _ => CloneError::Create(error),
    }
}
