//! Crediting-policy tests against a recording pool, plus the hardware-read failure classes.

use std::cell::RefCell;
use std::io::{self, Read};
use std::time::{Duration, Instant};

use super::{
    CONTRIBUTION_BYTES, CREDITED_BITS, EntropyError, FIRST_POLL, POLL, Pool, UNCREDITED_BITS,
    credit, kernel, polls, read_hardware,
};

/// Every contribution the policy handed to the kernel, in order.
#[derive(Debug, Default)]
struct Recorder {
    contributions: RefCell<Vec<(libc::c_int, [u8; CONTRIBUTION_BYTES])>>,
    reseeds: RefCell<usize>,
    mix_error: Option<EntropyError>,
    reseed_error: Option<EntropyError>,
}

impl Recorder {
    fn credited_bits(&self) -> libc::c_int {
        self.contributions
            .borrow()
            .iter()
            .map(|(bits, _)| *bits)
            .sum()
    }

    fn mixed(&self) -> Vec<[u8; CONTRIBUTION_BYTES]> {
        self.contributions
            .borrow()
            .iter()
            .map(|(_, bytes)| *bytes)
            .collect()
    }
}

impl Pool for Recorder {
    fn add(
        &self,
        credited_bits: libc::c_int,
        bytes: &[u8; CONTRIBUTION_BYTES],
    ) -> Result<(), EntropyError> {
        if let Some(error) = self.mix_error {
            return Err(error);
        }
        self.contributions
            .borrow_mut()
            .push((credited_bits, *bytes));
        Ok(())
    }

    fn reseed(&self) -> Result<(), EntropyError> {
        if let Some(error) = self.reseed_error {
            return Err(error);
        }
        *self.reseeds.borrow_mut() += 1;
        Ok(())
    }
}

/// A hardware device that answers each read with the next scripted result.
struct Scripted(Vec<io::Result<Vec<u8>>>);

impl Read for Scripted {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self.0.pop() {
            None => Ok(0),
            Some(Err(error)) => Err(error),
            Some(Ok(bytes)) => {
                let count = bytes.len().min(buffer.len());
                buffer[..count].copy_from_slice(&bytes[..count]);
                Ok(count)
            }
        }
    }
}

fn scripted(mut results: Vec<io::Result<Vec<u8>>>) -> Scripted {
    results.reverse();
    Scripted(results)
}

fn far() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

#[test]
fn only_the_fresh_hardware_read_is_credited() {
    let pool = Recorder::default();

    assert_eq!(
        credit(&[1; CONTRIBUTION_BYTES], &[2; CONTRIBUTION_BYTES], &pool),
        Ok(())
    );
    assert_eq!(
        *pool.contributions.borrow(),
        vec![
            (CREDITED_BITS, [1; CONTRIBUTION_BYTES]),
            (UNCREDITED_BITS, [2; CONTRIBUTION_BYTES]),
        ]
    );
    assert_eq!(UNCREDITED_BITS, 0);
    assert_eq!(usize::try_from(CREDITED_BITS), Ok(CONTRIBUTION_BYTES * 8));
    assert_eq!(pool.credited_bits(), CREDITED_BITS);
    assert_eq!(*pool.reseeds.borrow(), 1);
}

#[test]
fn a_zero_seed_is_mixed_without_raising_the_credited_bits() {
    let pool = Recorder::default();

    assert_eq!(
        credit(&[3; CONTRIBUTION_BYTES], &[0; CONTRIBUTION_BYTES], &pool),
        Ok(())
    );
    assert_eq!(pool.credited_bits(), CREDITED_BITS);
    assert_eq!(
        pool.mixed(),
        vec![[3; CONTRIBUTION_BYTES], [0; CONTRIBUTION_BYTES]]
    );
}

#[test]
fn a_repeated_seed_credits_no_more_than_the_first_repair() {
    let pool = Recorder::default();
    let replayed = [9; CONTRIBUTION_BYTES];

    assert_eq!(credit(&[4; CONTRIBUTION_BYTES], &replayed, &pool), Ok(()));
    let after_first = pool.credited_bits();
    assert_eq!(credit(&[5; CONTRIBUTION_BYTES], &replayed, &pool), Ok(()));

    assert_eq!(after_first, CREDITED_BITS);
    assert_eq!(pool.credited_bits(), CREDITED_BITS * 2);
    assert!(
        pool.contributions
            .borrow()
            .iter()
            .all(|(bits, bytes)| (*bytes == replayed) == (*bits == UNCREDITED_BITS))
    );
}

#[test]
fn a_rejected_contribution_reports_the_mix_failure_and_never_reseeds() {
    let pool = Recorder {
        mix_error: Some(EntropyError::Mix(libc::EPERM)),
        ..Recorder::default()
    };

    assert_eq!(
        credit(&[1; CONTRIBUTION_BYTES], &[2; CONTRIBUTION_BYTES], &pool),
        Err(EntropyError::Mix(libc::EPERM))
    );
    assert_eq!(*pool.reseeds.borrow(), 0);
}

#[test]
fn a_failed_reseed_is_reported_after_both_contributions_were_mixed() {
    let pool = Recorder {
        reseed_error: Some(EntropyError::Reseed(libc::EINVAL)),
        ..Recorder::default()
    };

    assert_eq!(
        credit(&[1; CONTRIBUTION_BYTES], &[2; CONTRIBUTION_BYTES], &pool),
        Err(EntropyError::Reseed(libc::EINVAL))
    );
    assert_eq!(pool.contributions.borrow().len(), 2);
    assert_eq!(*pool.reseeds.borrow(), 0);
}

#[test]
fn a_short_hardware_read_is_completed_across_calls() {
    let mut device = scripted(vec![
        Ok(vec![1; 1]),
        Ok(vec![2; 30]),
        Err(io::Error::from(io::ErrorKind::WouldBlock)),
        Ok(vec![3; CONTRIBUTION_BYTES]),
    ]);

    let bytes = read_hardware(&mut device, far()).expect("completed read");

    assert_eq!(bytes[0], 1);
    assert_eq!(bytes[1], 2);
    assert_eq!(bytes[30], 2);
    assert_eq!(bytes[31], 3);
    assert_eq!(bytes[CONTRIBUTION_BYTES - 1], 3);
}

#[test]
fn a_device_that_stops_early_is_unavailable_rather_than_a_partial_contribution() {
    let mut device = scripted(vec![Ok(vec![1; 32]), Ok(Vec::new())]);

    assert_eq!(
        read_hardware(&mut device, far()),
        Err(EntropyError::HardwareUnavailable(libc::ENODATA))
    );
}

#[test]
fn an_unavailable_hardware_device_reports_its_own_errno_classes() {
    let mut empty = scripted(Vec::new());
    let mut refusing = scripted(vec![Err(io::Error::from_raw_os_error(libc::EIO))]);
    let mut blocking = scripted(vec![Err(io::Error::from(io::ErrorKind::WouldBlock))]);
    let mut zeroed = scripted(vec![Ok(vec![0; CONTRIBUTION_BYTES])]);

    assert_eq!(
        read_hardware(&mut empty, far()),
        Err(EntropyError::HardwareUnavailable(libc::ENODATA))
    );
    assert_eq!(
        read_hardware(&mut refusing, far()),
        Err(EntropyError::HardwareUnavailable(libc::EIO))
    );
    assert_eq!(
        read_hardware(&mut blocking, Instant::now()),
        Err(EntropyError::HardwareUnavailable(libc::ETIMEDOUT))
    );
    assert_eq!(
        read_hardware(&mut zeroed, far()),
        Err(EntropyError::HardwareUnavailable(libc::EIO))
    );
}

#[test]
fn getrandom_is_nonblocking_once_the_repair_sequence_has_completed() {
    let pool = Recorder::default();

    assert_eq!(
        credit(&[6; CONTRIBUTION_BYTES], &[7; CONTRIBUTION_BYTES], &pool),
        Ok(())
    );
    assert_eq!(kernel::verify_nonblocking(), Ok(()));
}

#[test]
fn first_poll_is_the_shortest_and_the_sequence_saturates_at_the_ceiling() {
    assert!(FIRST_POLL < POLL);
    let taken: Vec<_> = polls().take(64).collect();
    assert_eq!(taken.first(), Some(&FIRST_POLL));
    assert!(
        taken.windows(2).all(|pair| pair[0] <= pair[1]),
        "the sequence must never wait less than it already did"
    );
    assert_eq!(
        taken.last(),
        Some(&POLL),
        "a device that stays empty must settle on the ceiling rather than grow past it"
    );
}
