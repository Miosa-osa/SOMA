//! Extent mapping through `FS_IOC_FIEMAP` so a clone can prove that its extents are shared.
//!
//! The request and extent layouts follow `include/uapi/linux/fiemap.h`; `libc` 0.2.189 does
//! not export them, so the crate defines the exact structures itself.

#![allow(unsafe_code)]

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};

/// `_IOWR('f', 11, struct fiemap)` from `include/uapi/linux/fiemap.h`.
///
/// The encoding is direction `3` in bits 30 and 31, the 32-byte header size in bits 16 to 29,
/// the `'f'` type in bits 8 to 15, and number 11 in the low byte.
pub const FS_IOC_FIEMAP: libc::Ioctl = 0xC020_660B;

/// `FIEMAP_FLAG_SYNC`: flush dirty data before mapping.
pub const FIEMAP_FLAG_SYNC: u32 = 0x0000_0001;
/// `FIEMAP_EXTENT_LAST`: this is the final extent of the file.
pub const FIEMAP_EXTENT_LAST: u32 = 0x0000_0001;
/// `FIEMAP_EXTENT_DELALLOC`: the extent has delayed allocation and no physical location yet.
pub const FIEMAP_EXTENT_DELALLOC: u32 = 0x0000_0004;
/// `FIEMAP_EXTENT_UNWRITTEN`: the extent is allocated but unwritten and reads as zero.
pub const FIEMAP_EXTENT_UNWRITTEN: u32 = 0x0000_0800;
/// `FIEMAP_EXTENT_SHARED`: the extent's blocks are shared with another file.
pub const FIEMAP_EXTENT_SHARED: u32 = 0x0000_2000;

/// Extents requested per `ioctl` call.
const BATCH_U32: u32 = 64;
/// [`BATCH_U32`] as a length.
const BATCH: usize = BATCH_U32 as usize;

/// `struct fiemap_extent` from `include/uapi/linux/fiemap.h`, 56 bytes, with the `fe_`
/// prefixes dropped.
#[repr(C)]
#[derive(Clone, Copy)]
struct FiemapExtent {
    logical: u64,
    physical: u64,
    length: u64,
    reserved64: [u64; 2],
    flags: u32,
    reserved: [u32; 3],
}

/// `struct fiemap` header from `include/uapi/linux/fiemap.h` with the `fm_` prefixes dropped,
/// followed by a fixed batch of extents, which matches the kernel's flexible array member
/// layout for `BATCH` entries.
#[repr(C)]
struct FiemapRequest {
    start: u64,
    length: u64,
    flags: u32,
    mapped_extents: u32,
    extent_count: u32,
    reserved: u32,
    extents: [FiemapExtent; BATCH],
}

const _: () = assert!(std::mem::size_of::<FiemapExtent>() == 56);
const _: () = assert!(std::mem::size_of::<FiemapRequest>() == 32 + 56 * BATCH);

/// Counts derived from one complete extent walk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExtentSummary {
    /// Extents reported by the filesystem.
    pub extents: u64,
    /// Extents carrying `FIEMAP_EXTENT_SHARED`.
    pub shared_extents: u64,
    /// Extents carrying `FIEMAP_EXTENT_UNWRITTEN`.
    pub unwritten_extents: u64,
    /// Extents carrying `FIEMAP_EXTENT_DELALLOC`.
    pub delalloc_extents: u64,
    /// Sum of extent lengths in bytes.
    pub mapped_bytes: u64,
}

impl ExtentSummary {
    /// True when the file has at least one extent and every extent is shared.
    #[must_use]
    pub fn all_shared(&self) -> bool {
        self.extents > 0 && self.shared_extents == self.extents
    }
}

/// Walks every extent of `fd` after flushing dirty data.
///
/// # Errors
///
/// Propagates the `ioctl` failure, including `EOPNOTSUPP` from filesystems without FIEMAP.
pub fn summarize(fd: BorrowedFd<'_>) -> io::Result<ExtentSummary> {
    let mut summary = ExtentSummary::default();
    let mut start = 0u64;
    loop {
        let mut request = FiemapRequest {
            start,
            length: u64::MAX,
            flags: FIEMAP_FLAG_SYNC,
            mapped_extents: 0,
            extent_count: BATCH_U32,
            reserved: 0,
            extents: [FiemapExtent {
                logical: 0,
                physical: 0,
                length: 0,
                reserved64: [0; 2],
                flags: 0,
                reserved: [0; 3],
            }; BATCH],
        };
        // SAFETY: `request` is a `repr(C)` value laid out exactly as the kernel's `struct
        // fiemap` followed by `extent_count` extents, it lives for the whole call, the
        // kernel writes at most `extent_count` entries, and `fd` is a live descriptor.
        let rc = unsafe { libc::ioctl(fd.as_raw_fd(), FS_IOC_FIEMAP, &raw mut request) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        let mapped = usize::try_from(request.mapped_extents)
            .map_err(|_| io::Error::other("fiemap extent count"))?
            .min(BATCH);
        if mapped == 0 {
            return Ok(summary);
        }
        let mut last_seen = false;
        for extent in &request.extents[..mapped] {
            summary.extents += 1;
            summary.mapped_bytes = summary.mapped_bytes.saturating_add(extent.length);
            if extent.flags & FIEMAP_EXTENT_SHARED != 0 {
                summary.shared_extents += 1;
            }
            if extent.flags & FIEMAP_EXTENT_UNWRITTEN != 0 {
                summary.unwritten_extents += 1;
            }
            if extent.flags & FIEMAP_EXTENT_DELALLOC != 0 {
                summary.delalloc_extents += 1;
            }
            if extent.flags & FIEMAP_EXTENT_LAST != 0 {
                last_seen = true;
            }
            let next = extent.logical.saturating_add(extent.length);
            if next <= start {
                return Err(io::Error::other("fiemap did not advance"));
            }
            start = next;
        }
        if last_seen {
            return Ok(summary);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_code_matches_the_uapi_encoding() {
        let direction = 3u64 << 30;
        let size = 32u64 << 16;
        let kind = u64::from(b'f') << 8;
        assert_eq!(FS_IOC_FIEMAP, direction | size | kind | 0x0b);
    }

    #[test]
    fn all_shared_requires_at_least_one_extent() {
        assert!(!ExtentSummary::default().all_shared());
        let shared = ExtentSummary {
            extents: 3,
            shared_extents: 3,
            ..ExtentSummary::default()
        };
        assert!(shared.all_shared());
        let partial = ExtentSummary {
            extents: 3,
            shared_extents: 2,
            ..ExtentSummary::default()
        };
        assert!(!partial.all_shared());
    }

    #[test]
    fn summarize_rejects_a_filesystem_without_fiemap_or_reports_extents() {
        use std::io::Write;
        let file = tempfile::tempfile().expect("tempfile");
        (&file).write_all(&[1u8; 8192]).expect("write");
        match summarize(std::os::fd::AsFd::as_fd(&file)) {
            Ok(summary) => assert!(summary.extents >= 1 || summary.delalloc_extents >= 1),
            Err(error) => assert_eq!(error.raw_os_error(), Some(libc::EOPNOTSUPP)),
        }
    }
}
