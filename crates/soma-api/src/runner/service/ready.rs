//! Waiting for a sandbox whose create answered before its launch finished.
//!
//! A create may answer as soon as its launch has taken a facade lease and keep bringing the
//! sandbox up behind the answer. Every call that then names that sandbox waits here first, so
//! nothing above this point has to know the launch was ever in flight: by the time a route
//! reaches its own checks the sandbox is either up, or gone with the answer the create would
//! have given for it.
//!
//! The wait is bounded by the launch itself, which is the same bound a client of a synchronous
//! create is under. There is deliberately no second deadline here: a launch that never ends is
//! exactly the stuck launch a synchronous create would be waiting on, and a clock of this
//! module's own could only disagree with it, leaving a machine running that a `201` had already
//! been sent for.

use std::time::{Duration, Instant};

use crate::runner::{ids::SandboxId, sandboxes::Unavailable, starting::LaunchState};

use super::{Runner, RunnerResponse, outcome};

impl Runner {
    /// Waits for a sandbox whose launch is still in flight, if it is.
    ///
    /// Returns how long this call waited, for the `wait` segment of its answer's timing, or the
    /// answer this call owes a launch that failed.
    ///
    /// A sandbox that is not starting, and one this call cannot address at all, are left to the
    /// route that named it: this decides nothing the routes already decide, except what a failed
    /// launch answers, which is the one case they cannot see for themselves.
    ///
    /// # Errors
    ///
    /// Returns the answer for a launch that failed after its create had answered: the same status
    /// and code the synchronous create would have given, which is the `503` a launch the engine
    /// refused answers with and the `429` a launch that could not take a facade lease answers
    /// with.
    pub(super) async fn wait_for_launch(
        &self,
        id: &SandboxId,
        tenant_id: &str,
    ) -> Result<Option<Duration>, Box<RunnerResponse>> {
        let mut launch = match self.sandboxes.in_flight(id, tenant_id) {
            Ok(Some(launch)) => launch,
            Err(Unavailable::LaunchFailed(failure)) => {
                return Err(Box::new(outcome::launch_failed(failure)));
            }
            Ok(None) | Err(_) => return Ok(None),
        };
        let waited = Instant::now();
        match launch.settled().await {
            LaunchState::Ready { .. } => Ok(Some(waited.elapsed())),
            LaunchState::Failed(code) => {
                eprintln!("soma-api: runner create {id} failed: {code}");
                Err(Box::new(RunnerResponse::create_unavailable()))
            }
            LaunchState::Lost => {
                // Nothing here can say whether a machine was left behind: the task that owned the
                // launch ended without reporting one way or the other. The operator has the id.
                eprintln!("soma-api: runner create {id} was lost before it reported");
                Err(Box::new(RunnerResponse::create_unavailable()))
            }
            LaunchState::Busy => Err(Box::new(RunnerResponse::runtime_busy())),
            // `settled` returns nothing else, so these arms are the type system's, not a
            // caller's: a launch that was still in flight would not have been returned.
            LaunchState::Pending | LaunchState::Launched => {
                Err(Box::new(RunnerResponse::create_unavailable()))
            }
        }
    }
}
