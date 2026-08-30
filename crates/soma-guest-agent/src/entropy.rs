//! Kernel entropy repair from fresh virtio-rng output and the launch-page seed.
//!
//! The captured snapshot contains the old kernel CSPRNG state, so repair mixes fresh material
//! into the kernel pool, forces an immediate CRNG reseed, and proves that `getrandom` no longer
//! blocks.
//!
//! # Crediting rule
//!
//! Exactly one contribution is credited: the fresh 64-byte `/dev/hwrng` read, which the virtio
//! entropy device sources from the Host for this boot.
//! The launch seed is mixed with a credit of zero.
//! Its confidentiality and its source are Host obligations that this guest cannot verify, and a
//! seed that was replayed from a captured page, zeroed, or chosen by whoever wrote the page
//! carries no entropy the kernel may count.
//! Mixing it can only help the pool; crediting it would let untrusted material raise the
//! kernel's entropy estimate and unblock `getrandom` on a predictable pool.
//!
//! The agent keeps no user-space PRNG: every later random value is a fresh `getrandom` call
//! made by Snow's operating-system resolver.

use std::io::{self, Read};
use std::thread;
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use crate::timings::{self, Step};

mod kernel;

#[cfg(test)]
mod tests;

/// Bytes in one entropy contribution: one hardware read, and one launch seed.
pub(crate) const CONTRIBUTION_BYTES: usize = 64;
/// Bits credited for the fresh hardware read.
const CREDITED_BITS: libc::c_int = 512;
/// Bits credited for the launch seed, which is mixed but never counted.
const UNCREDITED_BITS: libc::c_int = 0;
const POLL: Duration = Duration::from_millis(2);

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

/// The kernel entropy pool as the repair policy uses it.
trait Pool {
    /// Mixes `bytes` into the pool, crediting exactly `credited_bits` bits for them.
    fn add(
        &self,
        credited_bits: libc::c_int,
        bytes: &[u8; CONTRIBUTION_BYTES],
    ) -> Result<(), EntropyError>;

    /// Forces an immediate CRNG reseed from the pool.
    fn reseed(&self) -> Result<(), EntropyError>;
}

/// Reseeds the kernel from fresh hardware entropy and the launch seed, then verifies readiness.
///
/// # Errors
///
/// Returns the first failed step with its errno.
pub fn repair(seed: &[u8; CONTRIBUTION_BYTES], deadline: Instant) -> Result<(), EntropyError> {
    let hardware = timings::measure(Step::EntropyRead, || {
        read_hardware(&mut kernel::hardware_device()?, deadline)
    })?;
    let pool = kernel::KernelPool::open()?;
    timings::measure(Step::EntropyMix, || credit(&hardware, seed, &pool))?;
    timings::measure(Step::EntropyVerify, kernel::verify_nonblocking)
}

/// Mixes the credited hardware read and the uncredited launch seed, then forces a reseed.
fn credit(
    hardware: &[u8; CONTRIBUTION_BYTES],
    seed: &[u8; CONTRIBUTION_BYTES],
    pool: &impl Pool,
) -> Result<(), EntropyError> {
    pool.add(CREDITED_BITS, hardware)?;
    pool.add(UNCREDITED_BITS, seed)?;
    pool.reseed()
}

/// Fills one complete contribution from the nonblocking hardware device before the deadline.
fn read_hardware(
    device: &mut impl Read,
    deadline: Instant,
) -> Result<Zeroizing<[u8; CONTRIBUTION_BYTES]>, EntropyError> {
    let mut bytes = Zeroizing::new([0_u8; CONTRIBUTION_BYTES]);
    let mut filled = 0;
    while filled < CONTRIBUTION_BYTES {
        match device.read(&mut bytes[filled..]) {
            Ok(0) => return Err(EntropyError::HardwareUnavailable(libc::ENODATA)),
            Ok(count) => filled += count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(EntropyError::HardwareUnavailable(libc::ETIMEDOUT));
                }
                thread::sleep(POLL);
            }
            Err(error) => {
                return Err(EntropyError::HardwareUnavailable(kernel::errno_of(&error)));
            }
        }
    }
    if bytes.iter().all(|byte| *byte == 0) {
        return Err(EntropyError::HardwareUnavailable(libc::EIO));
    }
    Ok(bytes)
}
