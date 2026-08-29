//! Linux probe that proves a directory sits on XFS with working reflink support.
//!
//! The probe is the only startup check that may decide reflink support; every later clone
//! trusts the profile and treats a refused `FICLONE` as a fault rather than a fallback.

#![allow(unsafe_code)]

use std::ffi::CString;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

use super::{FilesystemKind, ProfileRejection, StorageProfile};
use crate::fiemap;

/// Bytes written to the probe source so the clone shares one real extent.
const PROBE_BYTES: usize = 4096;

impl StorageProfile {
    /// Probes the filesystem behind `dir` and proves reflink with one tiny `FICLONE`.
    ///
    /// The probe creates two private files named after the calling process inside `dir`,
    /// clones one into the other, checks that the destination shares its extent, and unlinks
    /// both before returning.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileRejection::NotXfs`] for another filesystem,
    /// [`ProfileRejection::ReflinkUnsupported`] when the kernel refuses the clone, and
    /// [`ProfileRejection::Probe`] for any other failure.
    pub fn probe(dir: BorrowedFd<'_>) -> Result<Self, ProfileRejection> {
        let (magic, block_size, free_bytes) =
            statfs_identity(dir).map_err(ProfileRejection::Probe)?;
        if magic != libc::XFS_SUPER_MAGIC {
            return Err(ProfileRejection::NotXfs { magic });
        }
        let (mount_id, device) = mount_identity(dir).map_err(ProfileRejection::Probe)?;
        probe_reflink(dir)?;
        Ok(Self {
            filesystem: FilesystemKind::XfsReflink,
            mount_id,
            device,
            block_size,
            free_bytes,
        })
    }
}

fn statfs_identity(dir: BorrowedFd<'_>) -> io::Result<(i64, u64, u64)> {
    // SAFETY: `statfs` is plain-old-data, so the all-zero value is a valid instance for the
    // kernel to overwrite, and `dir` is a live descriptor for the duration of the call.
    let stats = unsafe {
        let mut stats: libc::statfs = std::mem::zeroed();
        if libc::fstatfs(dir.as_raw_fd(), &raw mut stats) != 0 {
            return Err(io::Error::last_os_error());
        }
        stats
    };
    let block_size = u64::try_from(stats.f_bsize).map_err(|_| io::Error::other("f_bsize"))?;
    let free_bytes = block_size.saturating_mul(stats.f_bavail);
    Ok((stats.f_type, block_size, free_bytes))
}

fn mount_identity(dir: BorrowedFd<'_>) -> io::Result<(u64, u64)> {
    let empty = c"";
    // SAFETY: `statx` is plain-old-data, the all-zero value is valid for the kernel to fill,
    // `empty` is a NUL-terminated string that outlives the call, and `AT_EMPTY_PATH` makes
    // the kernel describe `dir` itself.
    let stats = unsafe {
        let mut stats: libc::statx = std::mem::zeroed();
        let rc = libc::statx(
            dir.as_raw_fd(),
            empty.as_ptr(),
            libc::AT_EMPTY_PATH,
            libc::STATX_MNT_ID,
            &raw mut stats,
        );
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        stats
    };
    if stats.stx_mask & libc::STATX_MNT_ID == 0 {
        return Err(io::Error::other("kernel did not report a mount id"));
    }
    let device = libc::makedev(stats.stx_dev_major, stats.stx_dev_minor);
    Ok((stats.stx_mnt_id, device))
}

/// Creates `name` under `dir` exclusively and returns the open descriptor.
fn create_exclusive(dir: BorrowedFd<'_>, name: &CString) -> io::Result<OwnedFd> {
    let flags = libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW;
    // SAFETY: `name` is NUL-terminated and outlives the call, `dir` is a live directory
    // descriptor, the flags request a new file, and the mode is the only extra argument the
    // `O_CREAT` form of `openat` reads; a non-negative result is a descriptor we now own.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags, 0o600 as libc::c_uint) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` was just returned by `openat` and nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn unlink(dir: BorrowedFd<'_>, name: &CString) {
    // SAFETY: `name` is NUL-terminated and outlives the call and `dir` is a live descriptor;
    // the result is deliberately ignored because the probe file may already be gone.
    let _ = unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) };
}

fn probe_reflink(dir: BorrowedFd<'_>) -> Result<(), ProfileRejection> {
    let pid = std::process::id();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let source_name = CString::new(format!(".soma-reflink-probe-{pid}-{nonce}-src"))
        .map_err(|error| ProfileRejection::Probe(io::Error::other(error)))?;
    let dest_name = CString::new(format!(".soma-reflink-probe-{pid}-{nonce}-dst"))
        .map_err(|error| ProfileRejection::Probe(io::Error::other(error)))?;

    let result = probe_pair(dir, &source_name, &dest_name);
    unlink(dir, &dest_name);
    unlink(dir, &source_name);
    result
}

fn probe_pair(
    dir: BorrowedFd<'_>,
    source_name: &CString,
    dest_name: &CString,
) -> Result<(), ProfileRejection> {
    let source = create_exclusive(dir, source_name).map_err(ProfileRejection::Probe)?;
    let payload = [0x5au8; PROBE_BYTES];
    // SAFETY: `payload` is a live buffer of exactly `PROBE_BYTES` bytes for the duration of
    // the call and `source` is a writable descriptor we own.
    let written = unsafe { libc::write(source.as_raw_fd(), payload.as_ptr().cast(), PROBE_BYTES) };
    if usize::try_from(written) != Ok(PROBE_BYTES) {
        return Err(ProfileRejection::Probe(io::Error::last_os_error()));
    }
    // SAFETY: `source` is a live descriptor we own.
    if unsafe { libc::fsync(source.as_raw_fd()) } != 0 {
        return Err(ProfileRejection::Probe(io::Error::last_os_error()));
    }
    let dest = create_exclusive(dir, dest_name).map_err(ProfileRejection::Probe)?;
    // SAFETY: `FICLONE` takes the source descriptor as its integer argument; both descriptors
    // are live and owned by this function for the duration of the call.
    let rc = unsafe { libc::ioctl(dest.as_raw_fd(), libc::FICLONE, source.as_raw_fd()) };
    if rc != 0 {
        let error = io::Error::last_os_error();
        return Err(match error.raw_os_error() {
            Some(libc::EOPNOTSUPP | libc::ENOTTY | libc::EXDEV | libc::EINVAL) => {
                ProfileRejection::ReflinkUnsupported
            }
            _ => ProfileRejection::Probe(error),
        });
    }
    let summary = fiemap::summarize(dest.as_fd()).map_err(ProfileRejection::Probe)?;
    if !summary.all_shared() {
        return Err(ProfileRejection::ReflinkUnsupported);
    }
    Ok(())
}
