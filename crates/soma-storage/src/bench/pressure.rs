//! Free-space pressure: fill the filesystem with preallocated filler files.

#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::path::{Path, PathBuf};

/// Largest single filler file.
const CHUNK_BYTES: u64 = 1024 * 1024 * 1024;

/// Filler files that must be removed after the pressured cell.
#[derive(Debug, Default)]
pub struct Filler {
    paths: Vec<PathBuf>,
    /// Bytes allocated by the filler.
    pub allocated_bytes: u64,
}

impl Filler {
    /// Number of filler files.
    #[must_use]
    pub fn files(&self) -> usize {
        self.paths.len()
    }

    /// Removes every filler file and syncs the directory.
    ///
    /// # Errors
    ///
    /// Returns the first removal failure.
    pub fn remove(self, dir: &Path) -> io::Result<()> {
        for path in &self.paths {
            std::fs::remove_file(path)?;
        }
        File::open(dir)?.sync_all()
    }
}

/// Total and available bytes of the filesystem behind `dir`.
///
/// # Errors
///
/// Propagates the `fstatfs` failure.
pub fn space(dir: BorrowedFd<'_>) -> io::Result<(u64, u64)> {
    // SAFETY: `statfs` is plain-old-data, the all-zero value is valid for the kernel to
    // overwrite, and `dir` is a live descriptor for the duration of the call.
    let stats = unsafe {
        let mut stats: libc::statfs = std::mem::zeroed();
        if libc::fstatfs(dir.as_raw_fd(), &raw mut stats) != 0 {
            return Err(io::Error::last_os_error());
        }
        stats
    };
    let block = u64::try_from(stats.f_bsize).map_err(|_| io::Error::other("f_bsize"))?;
    Ok((
        block.saturating_mul(stats.f_blocks),
        block.saturating_mul(stats.f_bavail),
    ))
}

/// Allocates filler files under `dir` until at most `target_free_percent` of the filesystem
/// is free.
///
/// # Errors
///
/// Returns the first creation or allocation failure other than `ENOSPC`.
pub fn fill(dir: &Path, dir_fd: BorrowedFd<'_>, target_free_percent: u64) -> io::Result<Filler> {
    let mut filler = Filler::default();
    let (total, _) = space(dir_fd)?;
    let target_free = total / 100 * target_free_percent;
    loop {
        let (_, available) = space(dir_fd)?;
        if available <= target_free {
            break;
        }
        let chunk = (available - target_free).min(CHUNK_BYTES);
        let path = dir.join(format!("filler-{}", filler.paths.len()));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        filler.paths.push(path);
        let length = libc::off_t::try_from(chunk).map_err(|_| io::Error::other("length"))?;
        // SAFETY: `file` is a live writable descriptor and mode 0 with offset 0 and a checked
        // length asks the kernel only to allocate space inside this new file.
        if unsafe { libc::fallocate(file.as_raw_fd(), 0, 0, length) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ENOSPC) {
                break;
            }
            return Err(error);
        }
        filler.allocated_bytes += chunk;
        if chunk < CHUNK_BYTES {
            break;
        }
    }
    File::open(dir)?.sync_all()?;
    Ok(filler)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsFd;

    #[test]
    fn space_reports_a_nonzero_total_for_a_temporary_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = File::open(temp.path()).expect("open");
        let (total, available) = space(dir.as_fd()).expect("statfs");
        assert!(total > 0);
        assert!(available <= total);
    }

    #[test]
    fn filling_to_the_current_free_level_allocates_nothing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = File::open(temp.path()).expect("open");
        let filler = fill(temp.path(), dir.as_fd(), 100).expect("fill");
        assert_eq!(filler.files(), 0);
        assert_eq!(filler.allocated_bytes, 0);
        filler.remove(temp.path()).expect("remove");
    }
}
