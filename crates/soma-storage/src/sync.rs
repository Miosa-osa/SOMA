//! One filesystem-wide durability barrier.
//!
//! `fsync` commits one file, so committing the writes of several files at once
//! takes one `syncfs` on the filesystem they live on. That is what lets a batch
//! of writers share a single durability barrier instead of each paying its own.

#![allow(unsafe_code)]

use std::{fs::File, io, os::fd::AsRawFd, path::Path};

/// Commits every write already made on the filesystem `directory` lives on.
///
/// The call also commits unrelated dirty data on that filesystem, which is
/// more than the caller needs and never less than it needs.
///
/// # Errors
///
/// Returns the operating system's error when the directory cannot be opened, or
/// the error `syncfs` reported.
pub fn filesystem(directory: &Path) -> io::Result<()> {
    let file = File::open(directory)?;
    // SAFETY: `syncfs` reads one open file descriptor and returns an error code.
    // It owns no memory and retains no reference past the call, and the
    // descriptor stays valid across it because `file` is alive until it returns.
    let outcome = unsafe { libc::syncfs(file.as_raw_fd()) };
    if outcome == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
