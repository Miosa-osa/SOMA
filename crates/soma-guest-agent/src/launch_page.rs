//! One-shot consumption of the fresh launch page.
//!
//! The VMM maps the 4 KiB page at [`LAUNCH_PAGE_GUEST_ADDRESS`], above RAM and the MMIO window.
//! The agent maps that guest-physical address through `/dev/mem`, waits at the disconnected
//! repair point until the page domain appears, copies the page once into locked zeroizing
//! memory, overwrites the mapping with zeroes, verifies the zeroes, and only then parses the
//! locked copy with the portable decoder.

#![allow(unsafe_code)]

use std::ptr;

use soma_guest::{GuestLaunchMaterial, LAUNCH_PAGE_SIZE};
use zeroize::Zeroizing;

pub use soma_guest::LAUNCH_PAGE_GUEST_ADDRESS;

/// Exact domain bytes at the start of every valid launch page.
pub const PAGE_DOMAIN: &[u8; 16] = b"SOMA-LAUNCH-PAGE";

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
pub fn consume(view: &mut [u8; LAUNCH_PAGE_SIZE]) -> Result<GuestLaunchMaterial, LaunchPageError> {
    let mut copy = Zeroizing::new([0_u8; LAUNCH_PAGE_SIZE]);
    let locked = Locked::pin(&mut copy)?;
    copy_volatile(view, locked.0);
    let erased = erase_and_verify(view);
    let parsed = GuestLaunchMaterial::take_from_page(locked.0);
    drop(locked);
    if !erased {
        return Err(LaunchPageError::EraseUnverified);
    }
    parsed.map_err(|_| LaunchPageError::Rejected)
}

/// Overwrites the view with zeroes through volatile stores and re-reads every byte.
#[must_use]
pub fn erase_and_verify(view: &mut [u8; LAUNCH_PAGE_SIZE]) -> bool {
    let base = view.as_mut_ptr();
    for index in 0..LAUNCH_PAGE_SIZE {
        // SAFETY: `index` is below the array length, so the offset pointer stays inside the
        // exclusively borrowed view and is valid for a one-byte volatile write and read.
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
pub fn page_present(view: &[u8; LAUNCH_PAGE_SIZE]) -> bool {
    let mut domain = [0; 16];
    for (index, byte) in domain.iter_mut().enumerate() {
        // SAFETY: `index` is below 16 and the view is at least 4096 bytes, so the pointer
        // stays inside the borrowed array for a one-byte volatile read.
        *byte = unsafe { ptr::read_volatile(view.as_ptr().add(index)) };
    }
    &domain == PAGE_DOMAIN
}

fn copy_volatile(source: &[u8; LAUNCH_PAGE_SIZE], destination: &mut [u8; LAUNCH_PAGE_SIZE]) {
    for (index, byte) in destination.iter_mut().enumerate() {
        // SAFETY: `index` is below the array length of the borrowed source.
        *byte = unsafe { ptr::read_volatile(source.as_ptr().add(index)) };
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
    use std::time::Duration;

    use soma_guest::{GuestLaunchMaterial, LAUNCH_PAGE_SIZE};

    use super::{LAUNCH_PAGE_GUEST_ADDRESS, LaunchPageError, consume, errno, page_present};

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

        fn view(&mut self) -> &mut [u8; LAUNCH_PAGE_SIZE] {
            // SAFETY: `base` is a live page-aligned mapping of exactly `LAUNCH_PAGE_SIZE` bytes
            // owned by `self`, and `&mut self` guarantees no other reference to it exists.
            unsafe { &mut *self.base.cast::<[u8; LAUNCH_PAGE_SIZE]>() }
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
        loop {
            if page_present(page.view()) {
                return consume(page.view());
            }
            thread::sleep(poll);
        }
    }
}

#[cfg(test)]
mod tests {
    use soma_guest::{HostLaunchMaterial, LaunchNetwork};

    use super::*;

    fn delivered_page() -> [u8; LAUNCH_PAGE_SIZE] {
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
        let mut page = [0xA5; LAUNCH_PAGE_SIZE];
        host.deliver_with(|bytes| {
            page.copy_from_slice(bytes);
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

        assert_eq!(view, [0; LAUNCH_PAGE_SIZE]);
        assert!(!page_present(&view));
        assert_eq!(material.binding().instance(), &[2; 16]);
        assert_eq!(material.network().vsock_cid(), 3);
    }

    #[test]
    fn a_malformed_page_is_rejected_but_still_erased() {
        let mut view = delivered_page();
        view[100] ^= 1;

        assert_eq!(
            consume(&mut view).expect_err("malformed page"),
            LaunchPageError::Rejected
        );
        assert_eq!(view, [0; LAUNCH_PAGE_SIZE]);
    }

    #[test]
    fn an_absent_page_is_not_present() {
        assert!(!page_present(&[0; LAUNCH_PAGE_SIZE]));
        assert!(!page_present(&[0xFF; LAUNCH_PAGE_SIZE]));
        assert_eq!(LAUNCH_PAGE_GUEST_ADDRESS, 0xd010_0000);
        assert_eq!(LAUNCH_PAGE_GUEST_ADDRESS % 4096, 0);
    }
}
