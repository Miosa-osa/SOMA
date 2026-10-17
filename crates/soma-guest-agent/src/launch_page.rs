//! One-shot consumption of the fresh launch page.
//!
//! The VMM maps the 4 KiB page at [`LAUNCH_PAGE_GUEST_ADDRESS`], above RAM and the MMIO window.
//! The agent maps that guest-physical address through `/dev/mem`, waits at the disconnected
//! repair point until the page domain appears, copies the page once into locked zeroizing
//! memory, overwrites the mapping with zeroes, verifies the zeroes, and only then parses the
//! locked copy with the portable decoder.

#![allow(unsafe_code)]

use std::time::Instant;

use soma_guest::{GuestLaunchMaterial, LAUNCH_PAGE_SIZE};
use zeroize::Zeroizing;

use crate::timings::{self, Step};

pub use soma_guest::LAUNCH_PAGE_GUEST_ADDRESS;

mod page;
#[cfg(test)]
mod tests;

pub use page::Page;

/// Redacted launch-page failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchPageError {
    /// The page failed identity, bound, or digest validation.
    Rejected,
    /// The page could not be verified as all zero after overwrite.
    EraseUnverified,
    /// The locked destination buffer could not be pinned.
    Lock(i32),
    /// `/dev/mem` could not be opened or mapped.
    Map(i32),
}

struct Locked<'a>(&'a mut [u8; LAUNCH_PAGE_SIZE]);

impl<'a> Locked<'a> {
    fn pin(page: &'a mut [u8; LAUNCH_PAGE_SIZE]) -> Result<Self, LaunchPageError> {
        // SAFETY: the pointer and length describe exactly the borrowed array, which outlives
        // the guard that unlocks it.
        let locked = unsafe { libc::mlock(page.as_ptr().cast(), LAUNCH_PAGE_SIZE) };
        if locked != 0 {
            return Err(LaunchPageError::Lock(errno()));
        }
        Ok(Self(page))
    }
}

impl Drop for Locked<'_> {
    fn drop(&mut self) {
        // SAFETY: the same borrowed array that was locked in `pin` is still alive here.
        unsafe { libc::munlock(self.0.as_ptr().cast(), LAUNCH_PAGE_SIZE) };
    }
}

/// Copies, erases, verifies, and parses one page view.
///
/// The view is always overwritten with zeroes, even when parsing fails.
///
/// # Errors
///
/// Returns a redacted failure and leaves no material behind.
pub fn consume(view: &mut Page) -> Result<GuestLaunchMaterial, LaunchPageError> {
    let mut copy = Zeroizing::new(Page::zeroed());
    let locked = Locked::pin(copy.bytes_mut())?;
    let pinned = Instant::now();
    view.copy_into(locked.0);
    let copied = Instant::now();
    let erased = view.erase_and_verify();
    let scrubbed = Instant::now();
    let parsed = GuestLaunchMaterial::take_from_page(locked.0);
    timings::record(Step::PageCopy, copied.saturating_duration_since(pinned));
    timings::record(Step::PageErase, scrubbed.saturating_duration_since(copied));
    timings::record(Step::PageParse, scrubbed.elapsed());
    drop(locked);
    if !erased {
        return Err(LaunchPageError::EraseUnverified);
    }
    parsed.map_err(|_| LaunchPageError::Rejected)
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

#[cfg(target_os = "linux")]
pub use physical::await_and_consume;

#[cfg(target_os = "linux")]
mod physical {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    use std::thread;
    use std::time::{Duration, Instant};

    use soma_guest::{GuestLaunchMaterial, LAUNCH_PAGE_SIZE};

    use crate::timings::{self, Step};

    use super::{LAUNCH_PAGE_GUEST_ADDRESS, LaunchPageError, Page, consume, errno};

    const DEV_MEM: &str = "/dev/mem";

    struct MappedPage {
        base: *mut u8,
    }

    impl MappedPage {
        fn map() -> Result<Self, LaunchPageError> {
            let device = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_SYNC | libc::O_CLOEXEC)
                .open(DEV_MEM)
                .map_err(|error| LaunchPageError::Map(error.raw_os_error().unwrap_or(0)))?;
            let offset = libc::off_t::try_from(LAUNCH_PAGE_GUEST_ADDRESS)
                .map_err(|_| LaunchPageError::Map(libc::EOVERFLOW))?;
            // SAFETY: a null hint, one page length, shared read-write protection, and a valid
            // open descriptor are the documented `mmap` operands; the result is checked below.
            let base = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    LAUNCH_PAGE_SIZE,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    device.as_raw_fd(),
                    offset,
                )
            };
            if base == libc::MAP_FAILED {
                return Err(LaunchPageError::Map(errno()));
            }
            Ok(Self { base: base.cast() })
        }

        #[expect(
            clippy::cast_ptr_alignment,
            reason = "mmap returns a page-aligned mapping, which is exactly the alignment Page \
                      asks for; the pointer's own type cannot say so"
        )]
        fn view(&mut self) -> &mut Page {
            // SAFETY: `base` is a live page-aligned mapping of exactly `LAUNCH_PAGE_SIZE` bytes
            // owned by `self`, and `&mut self` guarantees no other reference to it exists. A
            // whole mapped page is what `Page` describes, including its alignment.
            unsafe { &mut *self.base.cast::<Page>() }
        }
    }

    impl Drop for MappedPage {
        fn drop(&mut self) {
            // SAFETY: `base` and the length describe the mapping created in `map`.
            unsafe { libc::munmap(self.base.cast(), LAUNCH_PAGE_SIZE) };
        }
    }

    /// Waits at the disconnected repair point until the page appears, then consumes it.
    ///
    /// The wait is deliberately unbounded because the Generation snapshot is captured while the
    /// agent sits here; the host lifecycle bounds the restored guest from outside.
    ///
    /// # Errors
    ///
    /// Returns a redacted mapping, parsing, or erasure failure.
    pub fn await_and_consume(poll: Duration) -> Result<GuestLaunchMaterial, LaunchPageError> {
        let mut page = MappedPage::map()?;
        // A restore interrupts this sleep, so the wake that finds the page reports how much of
        // the pickup was the guest being asleep rather than the guest doing work.
        let mut slept = Instant::now();
        loop {
            let woke = Instant::now();
            let present = page.view().has_domain();
            let looked = Instant::now();
            if present {
                timings::record(Step::PollWake, woke.saturating_duration_since(slept));
                timings::record(Step::PageLook, looked.saturating_duration_since(woke));
                return consume(page.view());
            }
            slept = Instant::now();
            thread::sleep(poll);
        }
    }
}
