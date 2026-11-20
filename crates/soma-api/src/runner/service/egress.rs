//! Opening and closing the egress of a windowed sandbox.
//!
//! A windowed sandbox is created denied and holds a broker bundle, so a caller can open the public
//! internet for one call and shut it again. The guarantees here are the ones a caller leans on to
//! run untrusted work behind it:
//!
//! - An open answers only after the broker has put the rules in force.
//! - A failed open leaves the sandbox denied, and says so by closing again before it answers.
//! - A window is closed when it runs out, by this runner, whether or not the caller returns.
//! - A close that is not confirmed is retried by the sweep, and is reported as a failure, never
//!   as done.

use std::time::{Instant, SystemTime};

use serde_json::{Value, json};
use soma_local::{EgressSetFailure, EgressSetOutcome, EgressSetRequest};

use crate::runner::{
    clock::iso8601_micros,
    ids::SandboxId,
    keys::Principal,
    public_wire::PlatformError,
    sandboxes::{Held, Unavailable},
    windows::{Opening, Window, valid_window_id},
};

use super::{Runner, RunnerResponse, Timing, outcome};

mod sweep;
mod wire;

use wire::{failure_error, flip_failed, json_body, not_windowed, parse_open, window_body};

/// What one call to the host about a sandbox's egress came back with.
enum Flip {
    Done(EgressSetOutcome),
    Failed(EgressSetFailure),
    Busy,
}

impl Runner {
    /// `POST /api/v1/sandboxes/{id}/network/egress`
    pub(super) async fn open_egress(
        &self,
        principal: &Principal,
        raw_id: &str,
        body: &[u8],
        timing: &mut Timing,
    ) -> RunnerResponse {
        let wanted = match parse_open(body) {
            Ok(wanted) => wanted,
            Err(error) => return RunnerResponse::platform(&error),
        };
        let (id, instance, _held) = match self.egress_target(principal, raw_id, timing).await {
            Ok(target) => target,
            Err(response) => return *response,
        };
        if !self.windows.is_windowed(&id) {
            return RunnerResponse::platform(&not_windowed());
        }
        let window = Window {
            id: wanted.window_id.clone(),
            dns: wanted.dns,
            expires: Instant::now() + wanted.ttl,
            opening: false,
        };
        match self.windows.begin_open(&id, window) {
            Opening::Fresh => {}
            Opening::Already(open) => {
                return RunnerResponse::new(200, window_body(&id, &open, false));
            }
            Opening::InFlight => {
                return RunnerResponse::platform(&PlatformError::new(
                    409,
                    "egress_window_opening",
                    "this window is still being opened; ask again",
                    true,
                ));
            }
            Opening::Conflict(open) => {
                return RunnerResponse::platform(
                    &PlatformError::new(
                        409,
                        "egress_window_open",
                        "another window is open on this sandbox; close it first",
                        false,
                    )
                    .with_details(json!({"window_id": open.id})),
                );
            }
        }
        let flip = self
            .flip(
                &instance,
                EgressSetRequest {
                    open: true,
                    dns: wanted.dns,
                },
            )
            .await;
        match flip {
            Flip::Done(outcome) if outcome.open => {
                self.windows.opened(&id, &wanted.window_id);
                match self.windows.current(&id) {
                    Some(open) => {
                        RunnerResponse::new(200, window_body(&id, &open, outcome.changed))
                    }
                    None => RunnerResponse::platform(&flip_failed()),
                }
            }
            // The broker answered that the egress is not open, or did not answer: either way the
            // sandbox must end up denied, so it is closed again before the caller is told.
            other => {
                self.shut(&id, &instance).await;
                match other {
                    Flip::Failed(failure) => RunnerResponse::platform(&failure_error(failure)),
                    Flip::Busy => RunnerResponse::runtime_busy(),
                    Flip::Done(_) => RunnerResponse::platform(&flip_failed()),
                }
            }
        }
    }

    /// `DELETE /api/v1/sandboxes/{id}/network/egress/{window_id}`
    ///
    /// A close always asks the host to deny, whatever the runner remembers: it is idempotent, it
    /// flushes tracked flows each time, and a runner that lost its record still shuts the window.
    pub(super) async fn close_egress(
        &self,
        principal: &Principal,
        raw_id: &str,
        window_id: &str,
        timing: &mut Timing,
    ) -> RunnerResponse {
        if !valid_window_id(window_id) {
            return RunnerResponse::platform(&PlatformError::invalid_param(
                "window_id",
                "window_id must be 1 to 64 letters, digits, '-', '_' or '.'",
            ));
        }
        let (id, instance, _held) = match self.egress_target(principal, raw_id, timing).await {
            Ok(target) => target,
            Err(response) => return *response,
        };
        // Another window's id is not this window's to close.
        if let Some(open) = self.windows.current(&id)
            && open.id != window_id
        {
            return RunnerResponse::platform(
                &PlatformError::new(
                    409,
                    "egress_window_mismatch",
                    "a different window is open on this sandbox",
                    false,
                )
                .with_details(json!({"window_id": open.id})),
            );
        }
        let closed = EgressSetRequest {
            open: false,
            dns: false,
        };
        match self.flip(&instance, closed).await {
            Flip::Done(outcome) => {
                self.windows.closed(&id);
                RunnerResponse::new(
                    200,
                    json_body(&json!({
                        "sandbox_id": id.as_str(),
                        "window_id": window_id,
                        "egress": "denied",
                        "closed": true,
                        "changed": outcome.changed,
                    })),
                )
            }
            Flip::Failed(failure) => {
                // Not confirmed: stays recorded, due now, so the sweep keeps trying.
                self.windows.expire_now(&id);
                RunnerResponse::platform(&failure_error(failure))
            }
            Flip::Busy => {
                self.windows.expire_now(&id);
                RunnerResponse::runtime_busy()
            }
        }
    }

    /// `GET /api/v1/sandboxes/{id}/network`
    ///
    /// What the runner records: the network the create named and the window it opened. It is the
    /// runner's record, not a reading of the kernel; the close and the sweep are what enforce it.
    pub(super) async fn network(
        &self,
        principal: &Principal,
        raw_id: &str,
        timing: &mut Timing,
    ) -> RunnerResponse {
        let (id, _instance, _held) = match self.egress_target(principal, raw_id, timing).await {
            Ok(target) => target,
            Err(response) => return *response,
        };
        let windowed = self.windows.is_windowed(&id);
        let base = self.windows.base(&id);
        let window = self.windows.current(&id);
        let open = window.as_ref().filter(|window| !window.opening);
        let current = match (base, open) {
            (None, _) => Value::Null,
            (Some(_), Some(window)) => json!({"egress": "internet", "dns": window.dns}),
            (Some(base), None) => json!({"egress": base, "dns": base == "internet"}),
        };
        let expires_at = open.map(|window| {
            iso8601_micros(
                SystemTime::now() + window.expires.saturating_duration_since(Instant::now()),
            )
        });
        RunnerResponse::new(
            200,
            json_body(&json!({
                "sandbox_id": id.as_str(),
                "base": base.map(|egress| json!({"egress": egress, "windowed": windowed})),
                "current": current,
                "window_id": open.map(|window| window.id.as_str()),
                "expires_at": expires_at,
            })),
        )
    }

    /// Ask the host to deny this sandbox's egress, and record it only if the host confirmed.
    async fn shut(&self, id: &SandboxId, instance: &soma::InstanceId) {
        match self.flip(instance, CLOSED).await {
            Flip::Done(_) => self.windows.closed(id),
            // Not confirmed: the window stays recorded, due now, until a sweep confirms it.
            Flip::Failed(_) | Flip::Busy => self.windows.expire_now(id),
        }
    }

    async fn flip(&self, instance: &soma::InstanceId, request: EgressSetRequest) -> Flip {
        let instance = instance.clone();
        match self
            .backend
            .call(move |facade| facade.set_egress(&instance, request))
            .await
        {
            Ok((Ok(outcome), _)) => Flip::Done(outcome),
            Ok((Err(failure), _)) => Flip::Failed(failure),
            Err(_busy) => Flip::Busy,
        }
    }

    /// The sandbox a request names, once it is the caller's, up, and held for the call.
    async fn egress_target<'a>(
        &'a self,
        principal: &Principal,
        raw_id: &str,
        timing: &mut Timing,
    ) -> Result<(SandboxId, soma::InstanceId, Held<'a>), Box<RunnerResponse>> {
        let id = self.addressed(raw_id)?;
        timing.wait = self.wait_for_launch(&id, &principal.key.tenant_id).await?;
        let held = match self.sandboxes.hold(&id, &principal.key.tenant_id) {
            Ok(held) => held,
            Err(Unavailable::Busy) => {
                return Err(Box::new(RunnerResponse::platform(
                    &PlatformError::exec_busy(),
                )));
            }
            Err(Unavailable::NotFound | Unavailable::Destroyed(..)) => {
                return Err(Box::new(RunnerResponse::platform(
                    &PlatformError::sandbox_not_found(),
                )));
            }
            Err(Unavailable::LaunchFailed(failure)) => {
                return Err(Box::new(outcome::launch_failed(failure)));
            }
        };
        let instance = id.instance_id().ok_or_else(|| {
            Box::new(RunnerResponse::platform(&PlatformError::sandbox_not_found()))
        })?;
        Ok((id, instance, held))
    }
}

const CLOSED: EgressSetRequest = EgressSetRequest {
    open: false,
    dns: false,
};
