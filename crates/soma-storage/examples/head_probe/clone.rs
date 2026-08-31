//! The four syscalls one private head costs, each timed on its own.

use std::ffi::CString;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd, RawFd};

/// One thread's timings for one head, in microseconds.
#[derive(Clone, Copy, Default)]
pub struct Sample {
    /// Exclusive create of the destination under the head directory.
    pub create: f64,
    /// The `FICLONE` call.
    pub clone: f64,
    /// The `FIEMAP` walk that proves every extent is shared.
    pub verify: f64,
    /// Removal of the name, leaving the descriptor as the only reference.
    pub unlink: f64,
    /// Extents the verification walked.
    pub extents: u64,
}

impl Sample {
    /// Sum of every timed phase.
    #[must_use]
    pub fn total(&self) -> f64 {
        self.create + self.clone + self.verify + self.unlink
    }
}

/// Which phases run.
#[derive(Clone, Copy)]
pub struct Plan {
    /// Threads released together in one cohort.
    pub threads: usize,
    /// Cohorts in the run.
    pub cohorts: usize,
    /// Whether the `FICLONE` runs at all.
    pub do_clone: bool,
    /// Whether the `FIEMAP` verification runs.
    pub do_verify: bool,
    /// Whether each thread clones from its own reflink of the template.
    pub private_sources: bool,
}

const FS_IOC_FIEMAP: libc::Ioctl = 0xC020_660B;
const FIEMAP_FLAG_SYNC: u32 = 0x1;
const FIEMAP_EXTENT_LAST: u32 = 0x1;
const FIEMAP_EXTENT_SHARED: u32 = 0x2000;
const BATCH: usize = 64;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Extent {
    logical: u64,
    physical: u64,
    length: u64,
    reserved64: [u64; 2],
    flags: u32,
    reserved: [u32; 3],
}

#[repr(C)]
struct Request {
    start: u64,
    length: u64,
    flags: u32,
    mapped: u32,
    count: u32,
    reserved: u32,
    extents: [Extent; BATCH],
}

/// Walks every extent and returns the count, failing if any extent is not shared.
fn verify(fd: RawFd) -> u64 {
    let mut start = 0u64;
    let mut seen = 0u64;
    loop {
        let mut request = Request {
            start,
            length: u64::MAX,
            flags: FIEMAP_FLAG_SYNC,
            mapped: 0,
            count: u32::try_from(BATCH).expect("batch fits"),
            reserved: 0,
            extents: [Extent::default(); BATCH],
        };
        // SAFETY: `request` matches `struct fiemap` followed by `count` extents and outlives
        // the call; `fd` is live.
        let rc = unsafe { libc::ioctl(fd, FS_IOC_FIEMAP, &raw mut request) };
        assert!(rc == 0, "fiemap: {}", std::io::Error::last_os_error());
        let mapped = usize::try_from(request.mapped)
            .expect("count fits")
            .min(BATCH);
        if mapped == 0 {
            return seen;
        }
        for extent in &request.extents[..mapped] {
            assert!(
                extent.flags & FIEMAP_EXTENT_SHARED != 0,
                "extent not shared"
            );
            seen += 1;
        }
        let last = request.extents[mapped - 1];
        if last.flags & FIEMAP_EXTENT_LAST != 0 {
            return seen;
        }
        start = last.logical + last.length;
    }
}

fn create_exclusive(dir: RawFd, name: &CString) -> OwnedFd {
    let flags = libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW;
    // SAFETY: `name` is NUL terminated and outlives the call and `dir` is a live descriptor.
    let fd = unsafe { libc::openat(dir, name.as_ptr(), flags, 0o600 as libc::c_uint) };
    assert!(fd >= 0, "openat: {}", std::io::Error::last_os_error());
    // SAFETY: `fd` was just returned by `openat` and is owned by nobody else.
    unsafe { OwnedFd::from_raw_fd(fd) }
}

fn unlink(dir: RawFd, name: &CString) {
    // SAFETY: `name` is NUL terminated and outlives the call and `dir` is a live descriptor.
    let rc = unsafe { libc::unlinkat(dir, name.as_ptr(), 0) };
    assert!(rc == 0, "unlinkat: {}", std::io::Error::last_os_error());
}

/// Opens every entry of a comma separated path list read only.
#[must_use]
pub fn open_all(paths: &str) -> Vec<OwnedFd> {
    paths
        .split(',')
        .map(|path| {
            let file =
                std::fs::File::open(path).unwrap_or_else(|error| panic!("open {path}: {error}"));
            OwnedFd::from(file)
        })
        .collect()
}

/// Reflinks `count` private copies of the template so no two threads share a source inode.
#[must_use]
pub fn source_pool(template: RawFd, dir: RawFd, count: usize) -> Vec<OwnedFd> {
    (0..count)
        .map(|index| {
            let name = CString::new(format!("srcpool-{index}")).expect("name");
            let fd = create_exclusive(dir, &name);
            // SAFETY: both descriptors are live for the call.
            let rc = unsafe { libc::ioctl(fd.as_raw_fd(), libc::FICLONE, template) };
            assert!(rc == 0, "pool ficlone: {}", std::io::Error::last_os_error());
            unlink(dir, &name);
            fd
        })
        .collect()
}

/// Creates one private head and hands back its descriptor, still open and already unnamed.
#[must_use]
pub fn one(dir: RawFd, source: RawFd, index: usize, plan: &Plan) -> (Sample, OwnedFd) {
    use std::time::Instant;
    let mut sample = Sample::default();
    let name = CString::new(format!("probe-{index:04}")).expect("name");
    let mark = Instant::now();
    let fd = create_exclusive(dir, &name);
    sample.create = mark.elapsed().as_secs_f64() * 1e6;
    if plan.do_clone {
        let mark = Instant::now();
        // SAFETY: both descriptors are live for the duration of the call.
        let rc = unsafe { libc::ioctl(fd.as_raw_fd(), libc::FICLONE, source) };
        assert!(rc == 0, "ficlone: {}", std::io::Error::last_os_error());
        sample.clone = mark.elapsed().as_secs_f64() * 1e6;
    }
    if plan.do_verify && plan.do_clone {
        let mark = Instant::now();
        sample.extents = verify(fd.as_raw_fd());
        sample.verify = mark.elapsed().as_secs_f64() * 1e6;
    }
    let mark = Instant::now();
    unlink(dir, &name);
    sample.unlink = mark.elapsed().as_secs_f64() * 1e6;
    // The descriptor is returned rather than closed: a machine holds its head open for the
    // life of its sandbox, so a whole cohort's heads are live at once.
    (sample, fd)
}
