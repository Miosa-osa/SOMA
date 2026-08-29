//! Kernel entropy repair from fresh virtio-rng output and the launch-page seed.
//!
//! The captured snapshot contains the old kernel CSPRNG state, so repair credits fresh host
//! entropy through `RNDADDENTROPY`, forces an immediate CRNG reseed, and proves that
//! `getrandom` no longer blocks.
//! The agent keeps no user-space PRNG: every later random value is a fresh `getrandom` call
//! made by Snow's operating-system resolver.

#![allow(unsafe_code)]

use std::fs::OpenOptions;
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::thread;
use std::time::{Duration, Instant};

use zeroize::{Zeroize, Zeroizing};

use crate::ioctl;

const HWRNG_PATH: &str = "/dev/hwrng";
const RANDOM_PATH: &str = "/dev/random";
const HWRNG_BYTES: usize = 64;
const SEED_BYTES: usize = 64;
const POOL_WORDS: usize = (HWRNG_BYTES + SEED_BYTES) / 4;
const POOL_BYTES: libc::c_int = 128;
const CREDITED_BITS: libc::c_int = POOL_BYTES * 8;
const POLL: Duration = Duration::from_millis(2);
const RNDADDENTROPY: libc::c_ulong = 0x4008_5203;
const RNDRESEEDCRNG: libc::c_ulong = 0x5207;
const VERIFY_BYTES: usize = 32;

/// Redacted entropy-repair failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntropyError {
    /// The hardware random device produced no bytes before the deadline.
    HardwareUnavailable(i32),
    /// The kernel rejected the entropy contribution.
    Mix(i32),
    /// The kernel rejected the forced reseed.
    Reseed(i32),
    /// `getrandom` still blocks after repair.
    NotReady(i32),
}

/// Exact `struct rand_pool_info` layout for the fixed 128-byte contribution.
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

/// Reseeds the kernel from fresh hardware entropy and the launch seed, then verifies readiness.
///
/// # Errors
///
/// Returns the first failed step with its errno.
pub fn repair(seed: &[u8; SEED_BYTES], deadline: Instant) -> Result<(), EntropyError> {
    let hardware = read_hardware(deadline)?;
    let pool = Zeroizing::new(pool(&hardware, seed));
    let random = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(RANDOM_PATH)
        .map_err(|error| EntropyError::Mix(errno_of(&error)))?;
    // SAFETY: `RNDADDENTROPY` reads one `rand_pool_info` whose declared `buf_size` equals the
    // trailing buffer length; the structure is a valid local that outlives the call.
    let mixed = unsafe {
        libc::ioctl(
            random.as_raw_fd(),
            ioctl::request(RNDADDENTROPY),
            &raw const *pool,
        )
    };
    if mixed != 0 {
        return Err(EntropyError::Mix(last_errno()));
    }
    // SAFETY: `RNDRESEEDCRNG` takes no argument and only asks the kernel to reseed its CRNG.
    let reseeded = unsafe { libc::ioctl(random.as_raw_fd(), ioctl::request(RNDRESEEDCRNG)) };
    if reseeded != 0 {
        return Err(EntropyError::Reseed(last_errno()));
    }
    verify_nonblocking()
}

fn read_hardware(deadline: Instant) -> Result<Zeroizing<[u8; HWRNG_BYTES]>, EntropyError> {
    let mut device = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(HWRNG_PATH)
        .map_err(|error| EntropyError::HardwareUnavailable(errno_of(&error)))?;
    let mut bytes = Zeroizing::new([0_u8; HWRNG_BYTES]);
    let mut filled = 0;
    while filled < HWRNG_BYTES {
        match device.read(&mut bytes[filled..]) {
            Ok(0) => return Err(EntropyError::HardwareUnavailable(libc::ENODATA)),
            Ok(count) => filled += count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(EntropyError::HardwareUnavailable(libc::ETIMEDOUT));
                }
                thread::sleep(POLL);
            }
            Err(error) => return Err(EntropyError::HardwareUnavailable(errno_of(&error))),
        }
    }
    if bytes.iter().all(|byte| *byte == 0) {
        return Err(EntropyError::HardwareUnavailable(libc::EIO));
    }
    Ok(bytes)
}

fn pool(hardware: &[u8; HWRNG_BYTES], seed: &[u8; SEED_BYTES]) -> RandPoolInfo {
    let mut buf = [0_u32; POOL_WORDS];
    let combined: Vec<u8> = hardware.iter().chain(seed.iter()).copied().collect();
    for (word, chunk) in buf.iter_mut().zip(combined.as_chunks::<4>().0) {
        *word = u32::from_ne_bytes(*chunk);
    }
    RandPoolInfo {
        entropy_count: CREDITED_BITS,
        buf_size: POOL_BYTES,
        buf,
    }
}

fn verify_nonblocking() -> Result<(), EntropyError> {
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

fn errno_of(error: &io::Error) -> i32 {
    error.raw_os_error().unwrap_or(0)
}

fn last_errno() -> i32 {
    errno_of(&io::Error::last_os_error())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pool_credits_every_contributed_bit_with_the_kernel_layout() {
        let info = pool(&[1; HWRNG_BYTES], &[2; SEED_BYTES]);

        assert_eq!(info.entropy_count, 1024);
        assert_eq!(info.buf_size, 128);
        assert_eq!(usize::try_from(POOL_BYTES), Ok(POOL_WORDS * 4));
        assert_eq!(
            usize::try_from(CREDITED_BITS),
            Ok((HWRNG_BYTES + SEED_BYTES) * 8)
        );
        assert_eq!(std::mem::size_of::<RandPoolInfo>(), 8 + 128);
        assert_eq!(info.buf[0], u32::from_ne_bytes([1; 4]));
        assert_eq!(info.buf[POOL_WORDS - 1], u32::from_ne_bytes([2; 4]));
        assert_eq!(RNDADDENTROPY, 0x4008_5203);
        assert_eq!(RNDRESEEDCRNG, 0x5207);
    }

    #[test]
    fn the_host_kernel_random_source_is_already_nonblocking() {
        assert_eq!(verify_nonblocking(), Ok(()));
    }
}
