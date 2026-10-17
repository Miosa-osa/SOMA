//! `PATCH /api/v1/sandboxes/{id}` with `{"timeout": N}`: reset or extend the idle timer (C7).

use std::time::{Instant, SystemTime};

use serde::Serialize;
use serde_json::Value;

use crate::runner::{
    clock::iso8601_micros,
    idle::MAX_IDLE_TIMEOUT_SECONDS,
    keys::Principal,
    public_wire::{self, PlatformError},
    sandboxes::Unavailable,
};

use super::{Runner, RunnerResponse, Timing, outcome};

/// The answer to an extend, in the platform's sorted-key style.
#[derive(Serialize)]
struct Extended<'a> {
    /// When the sandbox ends unless touched again; null when nothing will end it.
    expires_at: Option<String>,
    id: &'a str,
    timeout_sec: u64,
}

impl Runner {
    /// A sandbox whose create answered before it was up is waited for here as well: an extend is
    /// an answer about a sandbox that exists, and it may not report a lifetime for one whose
    /// launch has not finished, let alone for one whose launch failed.
    pub(super) async fn extend(
        &self,
        principal: &Principal,
        raw_id: &str,
        body: &[u8],
        timing: &mut Timing,
    ) -> RunnerResponse {
        let id = match self.addressed(raw_id) {
            Ok(id) => id,
            Err(response) => return *response,
        };
        let waited = match self.wait_for_launch(&id, &principal.key.tenant_id).await {
            Ok(waited) => waited,
            Err(response) => return *response,
        };
        timing.wait = waited;
        let Some(seconds) = timeout(body) else {
            return RunnerResponse::platform(&PlatformError::invalid_param(
                "timeout",
                "timeout must be an integer between 0 and 86400 seconds; 0 is no idle timeout",
            ));
        };
        let idle = (seconds > 0).then(|| std::time::Duration::from_secs(seconds));
        match self.sandboxes.extend(&id, &principal.key.tenant_id, idle) {
            Ok(deadline) => {
                let now = Instant::now();
                let expires_at = deadline.map(|deadline| {
                    iso8601_micros(SystemTime::now() + deadline.saturating_duration_since(now))
                });
                RunnerResponse::new(
                    200,
                    public_wire::encode(&Extended {
                        expires_at,
                        id: id.as_str(),
                        timeout_sec: seconds,
                    }),
                )
            }
            Err(Unavailable::LaunchFailed(failure)) => outcome::launch_failed(failure),
            Err(_) => RunnerResponse::platform(&PlatformError::sandbox_not_found()),
        }
    }
}

fn timeout(body: &[u8]) -> Option<u64> {
    let document: Value = serde_json::from_slice(body).ok()?;
    document
        .get("timeout")?
        .as_u64()
        .filter(|seconds| *seconds <= MAX_IDLE_TIMEOUT_SECONDS)
}
