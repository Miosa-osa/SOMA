//! The public runner's request handling, independent of the HTTP version that carried it.

mod command;
mod create;
mod exec_contract;
#[cfg(test)]
mod exec_refusal_tests;
mod extend;
#[cfg(test)]
mod flow_tests;
mod forward;
#[cfg(test)]
mod idle_tests;
mod lifetime;
mod outcome;
#[cfg(test)]
mod outcome_tests;
mod params;
mod relay;
mod routing;
#[cfg(test)]
mod scope_tests;
mod shell;
mod stream;
#[cfg(test)]
mod stream_tests;
#[cfg(test)]
mod tests;
mod timing;

use std::{sync::Arc, time::Instant};

use bytes::Bytes;
use soma::OciImage;

use crate::runner::{
    backend::Backend,
    config::RunnerConfig,
    ids::SandboxId,
    journal::{Entry, EntryKind, Journal},
    keys::{KeyTable, Principal, Refusal},
    peers::Peers,
    public_wire::{self, PlatformError},
    rate_limit::RateLimiter,
    sandboxes::Sandboxes,
};

use command::failure_code;
use routing::{Route, route};
use timing::{Timing, millis};

/// The largest request body the runner reads: the loopback service's own bound, which a
/// maximal file write needs.
pub const MAX_BODY_BYTES: usize = crate::http::request::MAX_BODY_BYTES;
/// The tenant label a sandbox carries in the state store, so ownership survives a restart.
const TENANT_LABEL_PREFIX: &str = "t-";

/// One request, after the transport has read it, independent of HTTP version.
#[derive(Clone, Debug)]
pub struct RunnerRequest {
    pub method: http::Method,
    pub path: String,
    pub authorization: Option<String>,
    pub body: Bytes,
    pub received: Instant,
    /// Arrived from another runner over the private listener: never forwarded again.
    pub forwarded: bool,
}

/// One answer, plus the journal entry to write once it has been sent.
#[derive(Debug)]
pub struct RunnerResponse {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
    /// A body that is still being produced (server-sent events, a forwarded answer); sent
    /// after `body`, chunk by chunk, until the sender is dropped.
    pub stream: Option<tokio::sync::mpsc::Receiver<Bytes>>,
    pub journal: Option<Entry>,
}

impl RunnerResponse {
    fn new(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body,
            stream: None,
            journal: None,
        }
    }

    fn platform(error: &PlatformError) -> Self {
        Self::new(error.status, error.body())
    }

    fn refusal(status: u16, code: &str) -> Self {
        Self::new(status, public_wire::refusal(code))
    }

    /// The `503` that tells the SDK to retry on the next runner at once.
    /// `429 runtime_busy`: this runner is at its admission cap; the SDK retries the next IP.
    fn runtime_busy() -> Self {
        Self::refusal(429, "runtime_busy").header("retry-after", "0".to_owned())
    }

    /// `503 feed_stale`: this runner's key table is too old to admit a create.
    fn feed_stale() -> Self {
        Self::refusal(503, "feed_stale").header("retry-after", "0".to_owned())
    }

    fn header(mut self, name: &'static str, value: String) -> Self {
        self.headers.push((name, value));
        self
    }
}

/// The public runner: routing, admission, and the facade calls behind each route.
pub struct Runner {
    config: Arc<RunnerConfig>,
    keys: Arc<KeyTable>,
    sandboxes: Arc<Sandboxes>,
    limiter: RateLimiter,
    backend: Backend,
    journal: Journal,
    admission: tokio::sync::Semaphore,
    image: OciImage,
    peers: Option<Peers>,
}

impl Runner {
    /// # Panics
    ///
    /// Panics if the configured image does not parse; [`RunnerConfig::parse`] has already
    /// refused such a configuration.
    #[must_use]
    pub fn new(config: Arc<RunnerConfig>, backend: Backend, journal: Journal) -> Self {
        let image = OciImage::parse(config.launch.image.clone())
            .expect("the configuration validated the launch image");
        Self {
            admission: tokio::sync::Semaphore::new(config.admission()),
            keys: Arc::new(KeyTable::new()),
            sandboxes: Arc::new(Sandboxes::new()),
            limiter: RateLimiter::new(),
            config,
            backend,
            journal,
            image,
            peers: None,
        }
    }

    #[must_use]
    pub const fn keys(&self) -> &Arc<KeyTable> {
        &self.keys
    }

    #[must_use]
    pub const fn journal(&self) -> &Journal {
        &self.journal
    }

    #[must_use]
    pub fn sandboxes(&self) -> &Sandboxes {
        &self.sandboxes
    }

    /// Serves one request.
    pub async fn handle(&self, request: RunnerRequest) -> RunnerResponse {
        let mut timing = Timing::default();
        let mut response = self.dispatch(&request, &mut timing).await;
        if let Some(entry) = response.journal.as_mut() {
            entry.ms = millis(request.received.elapsed());
        }
        // A forwarded or streamed answer already carries the timing of the runner that ran it.
        if !response
            .headers
            .iter()
            .any(|(name, _)| *name == "server-timing")
        {
            response.headers.push(("server-timing", timing.header()));
        }
        response
    }

    async fn dispatch(&self, request: &RunnerRequest, timing: &mut Timing) -> RunnerResponse {
        let route = route(&request.method, &request.path);
        match route {
            Route::Health => return self.health(),
            Route::NotFound => return RunnerResponse::refusal(404, "not_found"),
            Route::Create
            | Route::Exec(_)
            | Route::ExecStream(_)
            | Route::Destroy(_)
            | Route::Extend(_)
            | Route::List
            | Route::Forward(_) => {}
        }
        let started = Instant::now();
        let principal = match self.keys.admit(request.authorization.as_deref()) {
            Ok(principal) => principal,
            Err(Refusal::Unauthorized) => return RunnerResponse::refusal(401, "unauthorized"),
            Err(Refusal::Forbidden) => return RunnerResponse::refusal(403, "forbidden"),
        };
        let limit = principal
            .key
            .rate_per_second
            .unwrap_or(self.config.rate_per_second);
        if !self.limiter.allow(&principal.key.key_id, limit, started) {
            return RunnerResponse::refusal(429, "rate_limited").header("retry-after", "1".into());
        }
        timing.auth = started.elapsed();
        if let Some(forwarded) = self.forward_to_owner(route, request).await {
            return forwarded;
        }
        match route {
            Route::Create => self.create(&principal, &request.body, timing).await,
            Route::Exec(id) => self.exec(&principal, id, &request.body, timing).await,
            Route::ExecStream(id) => {
                self.exec_stream(&principal, id, &request.body, request.received, timing)
            }
            Route::Destroy(id) => self.destroy(&principal, id, timing).await,
            Route::Extend(id) => self.extend(&principal, id, &request.body),
            Route::List => self.list(&principal, timing).await,
            Route::Forward(forward) => {
                self.forward(&principal, forward, &request.body, timing)
                    .await
            }
            Route::Health | Route::NotFound => RunnerResponse::refusal(404, "not_found"),
        }
    }

    fn health(&self) -> RunnerResponse {
        let age = millis(self.keys.feed_age(Instant::now()));
        // The prepared pool's depth is not observable through the facade; it is reported as
        // unknown rather than as a number that would mean something else.
        RunnerResponse::new(200, public_wire::health(self.config.host_tag, age, None))
    }

    /// Parses a path id and checks that this runner owns its tag.
    fn addressed(&self, raw_id: &str) -> Result<SandboxId, Box<RunnerResponse>> {
        let id = SandboxId::parse(raw_id)
            .ok_or_else(|| Box::new(RunnerResponse::platform(&PlatformError::invalid_id())))?;
        if id.tag() != self.config.host_tag {
            let runner_url = self.runner_url(id.tag());
            return Err(Box::new(RunnerResponse::new(
                421,
                public_wire::misdirected(&runner_url),
            )));
        }
        Ok(id)
    }

    /// `https://<tag>.<domain>`: where every call for a sandbox with this tag goes (C1).
    fn runner_url(&self, tag: char) -> String {
        format!("https://{tag}.{}", self.config.public_domain)
    }
}

fn entry(
    kind: EntryKind,
    principal: &Principal,
    project_id: Option<String>,
    sandbox: Option<&SandboxId>,
    status: u16,
) -> Entry {
    Entry {
        kind,
        tenant_id: principal.key.tenant_id.clone(),
        key_id: Some(principal.key.key_id.clone()),
        project_id,
        sandbox_id: sandbox.map(ToString::to_string),
        status,
        ms: 0,
        exit_code: None,
        cpu_ms: None,
        lifetime_ms: None,
        reason: None,
    }
}

/// Journals an id the caller could not address here: malformed ids are paperwork, a `421` is
/// not, because the runner that owns the sandbox journals the request it actually serves.
fn refused_sandbox(
    response: RunnerResponse,
    kind: EntryKind,
    principal: &Principal,
) -> RunnerResponse {
    if response.status == 421 {
        return response;
    }
    let status = response.status;
    with_journal(response, Some(entry(kind, principal, None, None, status)))
}

fn with_journal(mut response: RunnerResponse, entry: Option<Entry>) -> RunnerResponse {
    response.journal = entry;
    response
}
