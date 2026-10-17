//! One-shot consumption of the fresh launch page.
//!
//! The VMM maps the 4 KiB page at [`LAUNCH_PAGE_GUEST_ADDRESS`], above RAM and the MMIO window.
//! The agent maps that guest-physical address through `/dev/mem`, waits at the disconnected
//! repair point until the page domain appears, copies the page once into locked zeroizing
//! memory, overwrites the mapping with zeroes, verifies the zeroes, and only then parses the
//! locked copy with the portable decoder.

#![allow(unsafe_code)]

use std::ptr;
use std::time::Instant;

use soma_guest::{GuestLaunchMaterial, LAUNCH_PAGE_SIZE};
use zeroize::{Zeroize, Zeroizing};

use crate::timings::{self, Step};

pub use soma_guest::LAUNCH_PAGE_GUEST_ADDRESS;

/// Exact domain bytes at the start of every valid launch page.
pub const PAGE_DOMAIN: &[u8; 16] = b"SOMA-LAUNCH-PAGE";

/// The widest access this walks the page with.
type Word = u64;
/// Bytes one access covers, and how many of them the page holds.
const WORD_BYTES: usize = size_of::<Word>();
const WORDS: usize = LAUNCH_PAGE_SIZE / WORD_BYTES;

/// One launch page as the agent walks it: a whole number of words, aligned for one.
///
/// The page is mapped uncached, and an uncached access costs about the same whether it moves
/// one byte or eight, so the copy, the erase, and the verification after it walk the page a word
/// at a time. The alignment that walk needs is a property of this type rather than of whatever
/// the caller happened to pass, and the size is a whole number of words by construction.
#[repr(C, align(4096))]
pub struct Page([u8; LAUNCH_PAGE_SIZE]);

impl Page {
    /// A page of zeroes, which is what an erased page is.
    #[must_use]
    pub const fn zeroed() -> Self {
        Self([0; LAUNCH_PAGE_SIZE])
    }

    /// A page holding `byte` everywhere, for the tests that only care that it is not the domain.
    #[cfg(test)]
    fn filled(byte: u8) -> Self {
        Self([byte; LAUNCH_PAGE_SIZE])
    }

    /// The same bytes as a plain slice, for the locked copy the decoder reads.
    fn bytes_mut(&mut self) -> &mut [u8; LAUNCH_PAGE_SIZE] {
        &mut self.0
    }

    fn words(&self) -> *const Word {
        self.0.as_ptr().cast()
    }

    fn words_mut(&mut self) -> *mut Word {
        self.0.as_mut_ptr().cast()
    }
}

impl Zeroize for Page {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

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
    copy_volatile(view, locked.0);
    let copied = Instant::now();
    let erased = erase_and_verify(view);
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

/// Overwrites the view with zeroes through volatile stores and re-reads every word.
#[must_use]
pub fn erase_and_verify(view: &mut Page) -> bool {
    let base = view.words_mut();
    for index in 0..WORDS {
        // SAFETY: `Page` is aligned for `Word` and holds exactly `WORDS` of them, so `index`
        // stays inside the exclusively borrowed view and names one whole word.
        unsafe {
            ptr::write_volatile(base.add(index), 0);
            if ptr::read_volatile(base.add(index)) != 0 {
                return false;
            }
        }
    }
    true
}

/// Returns whether the view starts with the launch-page domain.
#[must_use]
pub fn page_present(view: &Page) -> bool {
    let source = view.words();
    let mut domain = [0_u8; PAGE_DOMAIN.len()];
    for (index, chunk) in domain
        .as_chunks_mut::<WORD_BYTES>()
        .0
        .iter_mut()
        .enumerate()
    {
        // SAFETY: the domain is fewer words than the page holds, so `index` stays inside the
        // borrowed view and names one whole word.
        let word = unsafe { ptr::read_volatile(source.add(index)) };
        chunk.copy_from_slice(&word.to_ne_bytes());
    }
    domain == *PAGE_DOMAIN
}

/// Copies the whole page one uncached word at a time into ordinary locked memory.
///
/// Only the source is accessed a word at a time: it is the uncached side, where an access costs
/// about the same whatever its width, so a byte-wise walk pays eight times the transactions for
/// the same bytes. The destination is locked anonymous memory, where a byte store is as cheap as
/// any other, so its bytes are written straight out of a register already holding the whole word.
fn copy_volatile(source: &Page, destination: &mut [u8; LAUNCH_PAGE_SIZE]) {
    let base = source.words();
    for index in 0..WORDS {
        // SAFETY: `Page` is aligned for `Word` and holds exactly `WORDS` of them, so `index`
        // stays inside the borrowed view and names one whole word.
        let word = unsafe { ptr::read_volatile(base.add(index)) };
        let start = index * WORD_BYTES;
        destination[start..start + WORD_BYTES].copy_from_slice(&word.to_ne_bytes());
    }
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

    use super::{LAUNCH_PAGE_GUEST_ADDRESS, LaunchPageError, Page, consume, errno, page_present};

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
            let present = page_present(page.view());
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

#[cfg(test)]
mod tests {
    use soma_guest::{HostLaunchMaterial, LaunchNetwork};

    use super::*;

    fn delivered_page() -> Page {
        let network = LaunchNetwork::new(
            3,
            1,
            [0x02, 0, 0, 0, 0, 1],
            [10, 0, 0, 2],
            24,
            [10, 0, 0, 1],
            [10, 0, 0, 1],
            1,
        )
        .expect("network");
        let host = HostLaunchMaterial::generate([1; 32], [2; 16], [3; 16], network).expect("host");
        let mut page = Page::filled(0xA5);
        host.deliver_with(|bytes| {
            page.0.copy_from_slice(bytes);
            Ok::<(), ()>(())
        })
        .expect("delivery");
        page
    }

    #[test]
    fn a_valid_page_is_consumed_and_the_view_is_zero_afterwards() {
        let mut view = delivered_page();
        assert!(page_present(&view));

        let material = consume(&mut view).expect("material");

        assert_eq!(view.0, [0; LAUNCH_PAGE_SIZE]);
        assert!(!page_present(&view));
        assert_eq!(material.binding().instance(), &[2; 16]);
        assert_eq!(material.network().vsock_cid(), 3);
    }

    #[test]
    fn a_malformed_page_is_rejected_but_still_erased() {
        let mut view = delivered_page();
        view.0[100] ^= 1;

        assert_eq!(
            consume(&mut view).expect_err("malformed page"),
            LaunchPageError::Rejected
        );
        assert_eq!(view.0, [0; LAUNCH_PAGE_SIZE]);
    }

    #[test]
    fn an_absent_page_is_not_present() {
        assert!(!page_present(&Page::zeroed()));
        assert!(!page_present(&Page::filled(0xFF)));
        assert_eq!(LAUNCH_PAGE_GUEST_ADDRESS, 0xd010_0000);
        assert_eq!(LAUNCH_PAGE_GUEST_ADDRESS % 4096, 0);
    }

    #[test]
    fn a_page_is_whole_words_of_bytes() {
        // The word walk is only sound because the page divides exactly, and the domain probe
        // only reads whole words because the domain does too.
        assert_eq!(LAUNCH_PAGE_SIZE % WORD_BYTES, 0);
        assert_eq!(PAGE_DOMAIN.len() % WORD_BYTES, 0);
        assert!(PAGE_DOMAIN.len() < LAUNCH_PAGE_SIZE);
        assert_eq!(
            std::mem::align_of::<Page>() % std::mem::align_of::<Word>(),
            0
        );
    }
}
