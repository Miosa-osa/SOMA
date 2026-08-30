//! The kernel-facing half of entropy repair: `/dev/hwrng`, `RNDADDENTROPY`, and `getrandom`.
//!
//! This module owns every entropy syscall so the crediting policy in the parent module stays
//! pure and testable against a recording pool.

#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;

use zeroize::{Zeroize, Zeroizing};

use crate::ioctl;

use super::{CONTRIBUTION_BYTES, EntropyError, Pool};

const HWRNG_PATH: &str = "/dev/hwrng";
const RANDOM_PATH: &str = "/dev/random";
const POOL_WORDS: usize = CONTRIBUTION_BYTES / 4;
const POOL_BYTES: libc::c_int = 64;
const RNDADDENTROPY: libc::c_ulong = 0x4008_5203;
const RNDRESEEDCRNG: libc::c_ulong = 0x5207;
const VERIFY_BYTES: usize = 32;

/// Exact `struct rand_pool_info` layout for one fixed 64-byte contribution.
#[repr(C)]
struct RandPoolInfo {
    entropy_count: libc::c_int,
    buf_size: libc::c_int,
    buf: [u32; POOL_WORDS],
}

impl Zeroize for RandPoolInfo {
    fn zeroize(&mut self) {
        self.entropy_count = 0;
        self.buf_size = 0;
        self.buf.zeroize();
    }
}

/// The live kernel entropy pool reached through `/dev/random`.
pub(super) struct KernelPool(File);

impl KernelPool {
    /// Opens the kernel pool for writing.
    pub(super) fn open() -> Result<Self, EntropyError> {
        OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_CLOEXEC)
            .open(RANDOM_PATH)
            .map(Self)
            .map_err(|error| EntropyError::Mix(errno_of(&error)))
    }
}

impl Pool for KernelPool {
    fn add(
        &self,
        credited_bits: libc::c_int,
        bytes: &[u8; CONTRIBUTION_BYTES],
    ) -> Result<(), EntropyError> {
        let info = Zeroizing::new(pool_info(credited_bits, bytes));
        // SAFETY: `RNDADDENTROPY` reads one `rand_pool_info` whose declared `buf_size` equals
        // the trailing buffer length; the structure is a valid local that outlives the call.
        let mixed = unsafe {
            libc::ioctl(
                self.0.as_raw_fd(),
                ioctl::request(RNDADDENTROPY),
                &raw const *info,
            )
        };
        if mixed == 0 {
            Ok(())
        } else {
            Err(EntropyError::Mix(last_errno()))
        }
    }

    fn reseed(&self) -> Result<(), EntropyError> {
        // SAFETY: `RNDRESEEDCRNG` takes no argument and only asks the kernel to reseed its CRNG.
        let reseeded = unsafe { libc::ioctl(self.0.as_raw_fd(), ioctl::request(RNDRESEEDCRNG)) };
        if reseeded == 0 {
            Ok(())
        } else {
            Err(EntropyError::Reseed(last_errno()))
        }
    }
}

/// Opens the hardware random device without blocking on an empty device.
pub(super) fn hardware_device() -> Result<File, EntropyError> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(HWRNG_PATH)
        .map_err(|error| EntropyError::HardwareUnavailable(errno_of(&error)))
}

/// Proves that `getrandom` returns a full sample without blocking.
pub(super) fn verify_nonblocking() -> Result<(), EntropyError> {
    let mut sample = Zeroizing::new([0_u8; VERIFY_BYTES]);
    // SAFETY: the buffer pointer and length describe the valid local array and the flag asks
    // the kernel to fail instead of blocking.
    let read = unsafe {
        libc::getrandom(
            sample.as_mut_ptr().cast(),
            VERIFY_BYTES,
            libc::GRND_NONBLOCK,
        )
    };
    if usize::try_from(read) == Ok(VERIFY_BYTES) {
        Ok(())
    } else {
        Err(EntropyError::NotReady(last_errno()))
    }
}

fn pool_info(credited_bits: libc::c_int, bytes: &[u8; CONTRIBUTION_BYTES]) -> RandPoolInfo {
    let mut buf = [0_u32; POOL_WORDS];
    for (word, chunk) in buf.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *word = u32::from_ne_bytes(*chunk);
    }
    RandPoolInfo {
        entropy_count: credited_bits,
        buf_size: POOL_BYTES,
        buf,
    }
}

pub(super) fn errno_of(error: &io::Error) -> i32 {
    error.raw_os_error().unwrap_or(0)
}

fn last_errno() -> i32 {
    errno_of(&io::Error::last_os_error())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_contribution_layout_matches_the_kernel_structure() {
        let info = pool_info(super::super::CREDITED_BITS, &[7; CONTRIBUTION_BYTES]);

        assert_eq!(std::mem::size_of::<RandPoolInfo>(), 8 + 64);
        assert_eq!(usize::try_from(POOL_BYTES), Ok(POOL_WORDS * 4));
        assert_eq!(usize::try_from(POOL_BYTES), Ok(CONTRIBUTION_BYTES));
        assert_eq!(info.buf_size, POOL_BYTES);
        assert_eq!(info.entropy_count, 512);
        assert_eq!(info.buf[0], u32::from_ne_bytes([7; 4]));
        assert_eq!(info.buf[POOL_WORDS - 1], u32::from_ne_bytes([7; 4]));
        assert_eq!(RNDADDENTROPY, 0x4008_5203);
        assert_eq!(RNDRESEEDCRNG, 0x5207);
    }

    #[test]
    fn an_uncredited_contribution_still_declares_the_full_buffer() {
        let info = pool_info(super::super::UNCREDITED_BITS, &[0; CONTRIBUTION_BYTES]);

        assert_eq!(info.entropy_count, 0);
        assert_eq!(info.buf_size, POOL_BYTES);
    }

    #[test]
    fn the_host_kernel_random_source_is_already_nonblocking() {
        assert_eq!(verify_nonblocking(), Ok(()));
    }
}
