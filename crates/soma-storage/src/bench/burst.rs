//! One burst: every thread creates one head after a shared barrier releases.

#![allow(unsafe_code)]

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Barrier;
use std::time::Instant;

use super::cell::Method;
use super::record::Sample;
use super::stats::nanos;
use super::templates::BenchTemplate;
use crate::clone;
use crate::fiemap;
use crate::head::HeadName;

/// The head directory as both a capability descriptor and, for the `cp` comparison, a path.
#[derive(Debug)]
pub struct HeadsDir {
    /// Open directory descriptor.
    pub file: File,
    /// Path used only by the subprocess method.
    pub path: PathBuf,
}

/// One unlink performed while a burst was running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnlinkSample {
    /// Thread index among the unlinkers.
    pub thread: usize,
    /// True when unlink and directory sync succeeded.
    pub ok: bool,
    /// Wall clock of unlink plus directory sync.
    pub total_ns: u64,
}

/// Everything one burst produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BurstOutcome {
    /// One sample per cloning thread.
    pub samples: Vec<Sample>,
    /// One sample per unlinking thread.
    pub unlinks: Vec<UnlinkSample>,
}

/// Runs one burst of `names.len()` cloners and `victims.len()` unlinkers released together.
#[must_use]
pub fn run_burst(
    method: Method,
    template: &BenchTemplate,
    dir: &HeadsDir,
    names: &[HeadName],
    victims: &[HeadName],
    burst: usize,
) -> BurstOutcome {
    let barrier = Barrier::new(names.len() + victims.len());
    let mut outcome = BurstOutcome::default();
    std::thread::scope(|scope| {
        let cloners: Vec<_> = names
            .iter()
            .enumerate()
            .map(|(thread, name)| {
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    match method {
                        Method::Ficlone => ficlone_sample(template, dir, name, burst, thread),
                        Method::CpReflink => cp_sample(template, dir, name, burst, thread),
                    }
                })
            })
            .collect();
        let unlinkers: Vec<_> = victims
            .iter()
            .enumerate()
            .map(|(thread, name)| {
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    unlink_sample(dir, name, thread)
                })
            })
            .collect();
        for handle in cloners {
            outcome.samples.push(handle.join().unwrap_or_else(|_| {
                Sample::failed(burst, usize::MAX, "cloner thread panicked".to_owned(), 0)
            }));
        }
        for handle in unlinkers {
            outcome.unlinks.push(handle.join().unwrap_or(UnlinkSample {
                thread: usize::MAX,
                ok: false,
                total_ns: 0,
            }));
        }
    });
    outcome
}

fn ficlone_sample(
    template: &BenchTemplate,
    dir: &HeadsDir,
    name: &HeadName,
    burst: usize,
    thread: usize,
) -> Sample {
    let wall = Instant::now();
    match clone::clone_head_timed(template.file.as_fd(), dir.file.as_fd(), name) {
        Ok((head, phases)) => {
            let wall_ns = nanos(wall.elapsed());
            let extents = head.extents();
            drop(head);
            Sample {
                burst,
                thread,
                ok: true,
                error: None,
                create_ns: nanos(phases.create),
                clone_ns: nanos(phases.clone),
                file_sync_ns: nanos(phases.file_sync),
                dir_sync_ns: nanos(phases.dir_sync),
                verify_ns: nanos(phases.verify),
                total_ns: nanos(phases.total()),
                wall_ns,
                extents: extents.extents,
                shared_extents: extents.shared_extents,
            }
        }
        Err(error) => Sample::failed(burst, thread, error.to_string(), nanos(wall.elapsed())),
    }
}

fn cp_sample(
    template: &BenchTemplate,
    dir: &HeadsDir,
    name: &HeadName,
    burst: usize,
    thread: usize,
) -> Sample {
    let wall = Instant::now();
    let destination = dir.path.join(name.as_str());
    let started = Instant::now();
    let status = Command::new("cp")
        .arg("--reflink=always")
        .arg(&template.path)
        .arg(&destination)
        .status();
    let clone_ns = nanos(started.elapsed());
    let status = match status {
        Ok(status) if status.success() => status,
        Ok(status) => {
            return Sample::failed(
                burst,
                thread,
                format!("cp exited with {status}"),
                nanos(wall.elapsed()),
            );
        }
        Err(error) => {
            return Sample::failed(
                burst,
                thread,
                format!("cp spawn failed: {error}"),
                nanos(wall.elapsed()),
            );
        }
    };
    let _ = status;
    let result = sync_and_verify(dir.file.as_fd(), &destination);
    let wall_ns = nanos(wall.elapsed());
    match result {
        Ok((file_sync_ns, dir_sync_ns, verify_ns, extents)) => Sample {
            burst,
            thread,
            ok: true,
            error: None,
            create_ns: 0,
            clone_ns,
            file_sync_ns,
            dir_sync_ns,
            verify_ns,
            total_ns: clone_ns + file_sync_ns + dir_sync_ns + verify_ns,
            wall_ns,
            extents: extents.extents,
            shared_extents: extents.shared_extents,
        },
        Err(error) => Sample::failed(burst, thread, error.to_string(), wall_ns),
    }
}

fn sync_and_verify(
    dir: BorrowedFd<'_>,
    destination: &std::path::Path,
) -> io::Result<(u64, u64, u64, fiemap::ExtentSummary)> {
    let file = File::open(destination)?;
    let started = Instant::now();
    clone::fsync(file.as_fd())?;
    let file_sync_ns = nanos(started.elapsed());
    let started = Instant::now();
    clone::fsync(dir)?;
    let dir_sync_ns = nanos(started.elapsed());
    let started = Instant::now();
    let extents = fiemap::summarize(file.as_fd())?;
    if !extents.all_shared() {
        return Err(io::Error::other(
            "cp destination extents are not all shared",
        ));
    }
    let verify_ns = nanos(started.elapsed());
    Ok((file_sync_ns, dir_sync_ns, verify_ns, extents))
}

fn unlink_sample(dir: &HeadsDir, name: &HeadName, thread: usize) -> UnlinkSample {
    let started = Instant::now();
    let ok = CString::new(name.as_str()).is_ok_and(|c_name| {
        // SAFETY: `c_name` is NUL-terminated and outlives the call and `dir.file` is a live
        // directory descriptor.
        unsafe { libc::unlinkat(dir.file.as_raw_fd(), c_name.as_ptr(), 0) == 0 }
    }) && clone::fsync(dir.file.as_fd()).is_ok();
    UnlinkSample {
        thread,
        ok,
        total_ns: nanos(started.elapsed()),
    }
}

/// Unlinks every name under `dir` and syncs the directory; absent names are not failures.
///
/// # Errors
///
/// Returns the first unlink failure other than absence, or the sync failure.
pub fn cleanup(dir: BorrowedFd<'_>, names: &[HeadName]) -> io::Result<()> {
    for name in names {
        let c_name = CString::new(name.as_str()).map_err(io::Error::other)?;
        // SAFETY: `c_name` is NUL-terminated and outlives the call and `dir` is a live
        // directory descriptor.
        if unsafe { libc::unlinkat(dir.as_raw_fd(), c_name.as_ptr(), 0) } != 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::NotFound {
                return Err(error);
            }
        }
    }
    clone::fsync(dir)
}

/// Flushes dirty pages and drops the page cache, dentries, and inodes.
///
/// # Errors
///
/// Propagates the write failure, which outside a privileged container is `EACCES`.
pub fn drop_caches() -> io::Result<()> {
    // SAFETY: `sync` takes no arguments and only schedules writeback.
    unsafe { libc::sync() };
    std::fs::write("/proc/sys/vm/drop_caches", b"3\n")
}
