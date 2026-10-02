//! A bound on how many creates this process works on at once.
//!
//! A create resumes a machine, and a resumed machine is a vCPU thread the host has to run. Past
//! the host's hardware thread count those vCPUs share cores, so every create in the burst gets
//! slower together and the commands behind them wait on CPU rather than on anything they asked
//! for. Measured on a 32-thread host with 50 creates at once, create rose from 5-7 ms to 21 ms
//! and a `true` command from 0.8 ms to 46 ms.
//!
//! A create over the bound is refused at once with `503 runtime_busy` and `Retry-After: 0`,
//! before any facade or machine is touched, so a caller with other runners retries there instead
//! of queueing here. Every other route is unaffected: an exec on an existing sandbox is never
//! refused because of creates.

use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use crate::envelope::ApiError;

/// The environment variable that sets the bound.
///
/// Absent means the host's available parallelism. A value that is not a positive integer is a
/// configuration error the service refuses to start with, rather than one it silently ignores.
pub const CREATE_ADMISSION_ENV: &str = "SOMA_API_MAX_CONCURRENT_CREATES";

/// The bound used when the host cannot report its parallelism.
///
/// It matches the service's default facade count, so an unknown host is held to the same bound it
/// already had rather than one that would refuse most of a burst.
const FALLBACK_LIMIT: NonZeroUsize = NonZeroUsize::new(128).expect("a positive constant");

/// How many creates may be in flight at once, shared by every connection thread.
#[derive(Clone, Debug)]
pub struct CreateAdmission {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    limit: usize,
    in_flight: AtomicUsize,
}

/// One admitted create. Dropping it frees its place.
#[derive(Debug)]
pub struct CreatePermit {
    inner: Arc<Inner>,
}

impl CreateAdmission {
    #[must_use]
    pub fn new(limit: NonZeroUsize) -> Self {
        Self {
            inner: Arc::new(Inner {
                limit: limit.get(),
                in_flight: AtomicUsize::new(0),
            }),
        }
    }

    /// The bound named by `configured`, the value of [`CREATE_ADMISSION_ENV`] if it is set.
    ///
    /// Returns `None` for a value that is set but is not a positive integer.
    #[must_use]
    pub fn from_setting(configured: Option<&str>) -> Option<Self> {
        let limit = match configured {
            None => std::thread::available_parallelism().unwrap_or(FALLBACK_LIMIT),
            Some(value) => value.trim().parse::<NonZeroUsize>().ok()?,
        };
        Some(Self::new(limit))
    }

    #[must_use]
    pub fn limit(&self) -> usize {
        self.inner.limit
    }

    /// Takes one place without waiting, or reports that every place is taken.
    #[must_use]
    pub fn try_admit(&self) -> Option<CreatePermit> {
        self.inner
            .in_flight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |taken| {
                (taken < self.inner.limit).then_some(taken + 1)
            })
            .ok()
            .map(|_| CreatePermit {
                inner: Arc::clone(&self.inner),
            })
    }

    /// The refusal a create over the bound receives.
    #[must_use]
    pub const fn busy() -> ApiError {
        ApiError::new(
            503,
            "runtime_busy",
            "this runner is at its concurrent create limit; retry on another runner",
            true,
        )
    }
}

impl Drop for CreatePermit {
    fn drop(&mut self) {
        self.inner.in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::CreateAdmission;

    fn admission(limit: usize) -> CreateAdmission {
        CreateAdmission::new(NonZeroUsize::new(limit).expect("a positive test limit"))
    }

    #[test]
    fn admits_up_to_the_limit_and_refuses_the_next() {
        let admission = admission(2);
        let first = admission.try_admit();
        let second = admission.try_admit();

        assert!(first.is_some() && second.is_some());
        assert!(admission.try_admit().is_none());
    }

    #[test]
    fn a_finished_create_frees_its_place() {
        let admission = admission(1);
        let held = admission.try_admit().expect("the first create is admitted");
        assert!(admission.try_admit().is_none());

        drop(held);

        assert!(admission.try_admit().is_some());
    }

    #[test]
    fn concurrent_callers_never_exceed_the_limit() {
        let admission = admission(4);
        let admitted = std::thread::scope(|scope| {
            let callers: Vec<_> = (0..32)
                .map(|_| scope.spawn(|| admission.try_admit()))
                .collect();
            callers
                .into_iter()
                .map(|caller| caller.join().expect("caller completed"))
                .collect::<Vec<_>>()
        });

        assert_eq!(admitted.iter().filter(|permit| permit.is_some()).count(), 4);
    }

    #[test]
    fn an_absent_setting_uses_the_host_parallelism() {
        let expected = std::thread::available_parallelism().map_or(128, NonZeroUsize::get);

        let admission = CreateAdmission::from_setting(None).expect("absent is valid");

        assert_eq!(admission.limit(), expected);
    }

    #[test]
    fn a_positive_setting_is_the_limit() {
        let admission = CreateAdmission::from_setting(Some(" 24 ")).expect("24 is valid");

        assert_eq!(admission.limit(), 24);
    }

    #[test]
    fn a_zero_or_malformed_setting_is_refused() {
        for value in ["0", "", "-1", "many", "1.5"] {
            assert!(
                CreateAdmission::from_setting(Some(value)).is_none(),
                "{value}"
            );
        }
    }

    #[test]
    fn the_refusal_is_a_retryable_503_runtime_busy() {
        let busy = CreateAdmission::busy();

        assert_eq!(busy.status(), 503);
        assert_eq!(busy.body().code, "runtime_busy");
        assert!(busy.body().retryable);
    }
}
