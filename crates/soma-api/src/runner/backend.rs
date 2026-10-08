use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{envelope::ApiError, facade::SandboxFacade};

/// Opens one facade for one request: in production a lease from the preopened pool the loopback
/// service also draws from, in tests a fake.
pub type FacadeOpener =
    Arc<dyn Fn() -> Result<Box<dyn SandboxFacade>, ApiError> + Send + Sync + 'static>;

/// The facade could not be leased within its bounded wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Busy;

/// What one facade call cost, split the way `Server-Timing` reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallTiming {
    /// Waiting for a facade lease.
    pub pool: Duration,
    /// The facade call itself.
    pub exec: Duration,
}

/// Runs blocking facade calls off the asynchronous workers.
///
/// Every facade call is a blocking call into the local runtime, so each runs on the blocking
/// pool and holds its lease only for that call, exactly as one loopback request does.
#[derive(Clone)]
pub struct Backend {
    opener: FacadeOpener,
}

impl Backend {
    #[must_use]
    pub fn new(opener: FacadeOpener) -> Self {
        Self { opener }
    }

    /// Leases a facade, runs `call` on it, and reports where the time went.
    ///
    /// # Errors
    ///
    /// Returns [`Busy`] when no facade could be leased, or when the blocking task itself was
    /// lost, which leaves the caller in the same position: nothing ran that it can report.
    pub async fn call<R: Send + 'static>(
        &self,
        call: impl FnOnce(&mut dyn SandboxFacade) -> R + Send + 'static,
    ) -> Result<(R, CallTiming), Busy> {
        let opener = Arc::clone(&self.opener);
        let queued = Instant::now();
        tokio::task::spawn_blocking(move || {
            let mut facade = opener().map_err(|_| Busy)?;
            let leased = Instant::now();
            let result = call(facade.as_mut());
            let finished = Instant::now();
            drop(facade);
            Ok((
                result,
                CallTiming {
                    pool: leased.saturating_duration_since(queued),
                    exec: finished.saturating_duration_since(leased),
                },
            ))
        })
        .await
        .map_err(|_| Busy)?
    }

    /// Runs `call` on a facade from synchronous code, for startup work done before serving.
    ///
    /// # Errors
    ///
    /// Returns [`Busy`] when no facade could be leased.
    pub fn call_now<R>(&self, call: impl FnOnce(&mut dyn SandboxFacade) -> R) -> Result<R, Busy> {
        let mut facade = (self.opener)().map_err(|_| Busy)?;
        Ok(call(facade.as_mut()))
    }
}
