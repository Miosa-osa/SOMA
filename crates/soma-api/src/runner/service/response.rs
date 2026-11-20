//! The answers every route builds its response from.
//!
//! A refusal, a busy runner, a stale key feed, and a create that could not claim a sandbox are
//! answered the same way wherever they happen, so their constructors live in one place.

use super::{PlatformError, RunnerResponse, public_wire};

impl RunnerResponse {
    pub(super) fn new(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body,
            stream: None,
            journal: None,
            tunnel: None,
        }
    }

    pub(super) fn platform(error: &PlatformError) -> Self {
        Self::new(error.status, error.body())
    }

    pub(super) fn refusal(status: u16, code: &str) -> Self {
        Self::new(status, public_wire::refusal(code))
    }

    /// The `503` that tells the SDK to retry on the next runner at once.
    /// `429 runtime_busy`: this runner is at its admission cap; the SDK retries the next IP.
    pub(super) fn runtime_busy() -> Self {
        Self::refusal(429, "runtime_busy").header("retry-after", "0".to_owned())
    }

    /// `503 feed_stale`: this runner's key table is too old to admit a create, or no snapshot has
    /// filled it yet, so no call can be judged.
    pub(super) fn feed_stale() -> Self {
        Self::refusal(503, "feed_stale").header("retry-after", "0".to_owned())
    }

    /// `503`: this runner could not claim a prepared sandbox.
    ///
    /// One constructor for the whole service, because a create's own failure and the same
    /// failure read later by a call on a sandbox whose launch failed after its create had
    /// answered must be the same answer, byte for byte.
    pub(super) fn create_unavailable() -> Self {
        Self::new(503, public_wire::create_unavailable()).header("retry-after", "1".to_owned())
    }

    pub(super) fn header(mut self, name: &'static str, value: String) -> Self {
        self.headers.push((name, value));
        self
    }
}
