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

use std::time::{Duration, Instant, SystemTime};

use serde_json::{Value, json};
use soma_local::{EgressSetFailure, EgressSetOutcome, EgressSetRequest};

use crate::runner::{
    clock::iso8601_micros,
    ids::SandboxId,
    keys::Principal,
    public_wire::PlatformError,
    sandboxes::{Held, Unavailable},
    windows::{MAX_WINDOW, Opening, Window, valid_window_id},
};

use super::{Runner, RunnerResponse, Timing, outcome};

/// What one call to the host about a sandbox's egress came back with.
enum Flip {
    Done(EgressSetOutcome),
    Failed(EgressSetFailure),
    Busy,
}

/// A window request, after validation.
struct OpenRequest {
    window_id: String,
    ttl: Duration,
    dns: bool,
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
            Opening::Already(open) => return RunnerResponse::new(200, window_body(&id, &open, false)),
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
                    Some(open) => RunnerResponse::new(200, window_body(&id, &open, outcome.changed)),
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
            iso8601_micros(SystemTime::now() + window.expires.saturating_duration_since(Instant::now()))
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

    /// Closes every window that has run out. A close the host does not confirm is tried again on
    /// the next sweep; a sandbox that is gone has nothing left to close.
    pub(super) async fn close_expired_windows(&self) {
        for id in self.windows.expired(Instant::now()) {
            let Some(instance) = id.instance_id() else {
                self.windows.forget(&id);
                continue;
            };
            match self.flip(&instance, CLOSED).await {
                Flip::Done(_) => self.windows.closed(&id),
                Flip::Failed(EgressSetFailure::NotFound) => self.windows.forget(&id),
                Flip::Failed(failure) => {
                    eprintln!("soma-api: runner could not close the egress window of {id}: {failure:?}");
                }
                Flip::Busy => {}
            }
        }
    }

    /// After a restart the runner does not know which windows were open, and the timers died with
    /// it. Asking every recovered sandbox to deny closes whatever was open; a sandbox that holds
    /// no window answers that there is nothing to change, which is ignored. The ones that
    /// answered are windowed, and are recorded as such.
    pub(super) async fn close_recovered_windows(&self, recovered: &[SandboxId]) {
        for id in recovered {
            let Some(instance) = id.instance_id() else {
                continue;
            };
            if let Flip::Done(_) = self.flip(&instance, CLOSED).await {
                self.windows.created(id, "denied", true);
            }
        }
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
                return Err(Box::new(RunnerResponse::platform(&PlatformError::exec_busy())));
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

fn parse_open(body: &[u8]) -> Result<OpenRequest, PlatformError> {
    let invalid = |field: &str, detail: &str| PlatformError::invalid_param(field, detail);
    let document: Value = serde_json::from_slice(body)
        .map_err(|_| invalid("body", "the body must be a JSON object"))?;
    let Value::Object(fields) = document else {
        return Err(invalid("body", "the body must be a JSON object"));
    };
    if let Some(unknown) = fields
        .keys()
        .find(|key| !matches!(key.as_str(), "egress" | "ttl_ms" | "window_id" | "dns"))
    {
        return Err(invalid(unknown, "unknown field"));
    }
    if fields.get("egress").and_then(Value::as_str) != Some("internet") {
        return Err(invalid("egress", "egress must be \"internet\""));
    }
    let window_id = fields
        .get("window_id")
        .and_then(Value::as_str)
        .filter(|id| valid_window_id(id))
        .ok_or_else(|| {
            invalid(
                "window_id",
                "window_id must be 1 to 64 letters, digits, '-', '_' or '.'",
            )
        })?
        .to_owned();
    let ttl = fields
        .get("ttl_ms")
        .and_then(Value::as_u64)
        .map(Duration::from_millis)
        .filter(|ttl| !ttl.is_zero() && *ttl <= MAX_WINDOW)
        .ok_or_else(|| {
            invalid(
                "ttl_ms",
                "ttl_ms must be between 1 and 900000 (fifteen minutes)",
            )
        })?;
    let dns = match fields.get("dns") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(dns)) => *dns,
        Some(_) => return Err(invalid("dns", "dns must be a boolean")),
    };
    Ok(OpenRequest {
        window_id,
        ttl,
        dns,
    })
}

fn window_body(id: &SandboxId, window: &Window, changed: bool) -> Vec<u8> {
    let expires_at = iso8601_micros(
        SystemTime::now() + window.expires.saturating_duration_since(Instant::now()),
    );
    json_body(&json!({
        "sandbox_id": id.as_str(),
        "window_id": window.id,
        "egress": "internet",
        "dns": window.dns,
        "changed": changed,
        "expires_at": expires_at,
    }))
}

fn json_body(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

fn not_windowed() -> PlatformError {
    PlatformError::new(
        409,
        "egress_not_windowed",
        "this sandbox was not created with network.windowed, so its egress cannot be opened",
        false,
    )
}

fn flip_failed() -> PlatformError {
    PlatformError::new(
        502,
        "egress_flip_failed",
        "the network rules could not be put in force; treat egress as unchanged for an open and as possibly open for a close",
        false,
    )
}

fn failure_error(failure: EgressSetFailure) -> PlatformError {
    match failure {
        EgressSetFailure::NotFound => PlatformError::sandbox_not_found(),
        EgressSetFailure::NotWindowed => not_windowed(),
        EgressSetFailure::NotActive => PlatformError::new(
            409,
            "egress_not_active",
            "the sandbox's network is not active yet",
            true,
        ),
        EgressSetFailure::FlipFailed => flip_failed(),
        EgressSetFailure::Unavailable => PlatformError::new(
            503,
            "egress_unavailable",
            "the sandbox's host could not be reached to change its network",
            true,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(body: &str) -> Result<OpenRequest, PlatformError> {
        parse_open(body.as_bytes())
    }

    #[test]
    fn a_window_request_names_its_id_and_a_bounded_ttl() {
        let parsed =
            open(r#"{"egress":"internet","ttl_ms":30000,"window_id":"call-1"}"#).expect("valid");
        assert_eq!(parsed.window_id, "call-1");
        assert_eq!(parsed.ttl, Duration::from_secs(30));
        assert!(parsed.dns, "dns defaults to on with the window");

        let without_dns =
            open(r#"{"egress":"internet","ttl_ms":1,"window_id":"c","dns":false}"#).expect("valid");
        assert!(!without_dns.dns);
    }

    #[test]
    fn anything_else_is_refused_not_served_as_a_default() {
        for body in [
            "",
            "[]",
            r#"{"ttl_ms":30000,"window_id":"w"}"#,
            r#"{"egress":"denied","ttl_ms":30000,"window_id":"w"}"#,
            r#"{"egress":"unrestricted","ttl_ms":30000,"window_id":"w"}"#,
            r#"{"egress":"internet","window_id":"w"}"#,
            r#"{"egress":"internet","ttl_ms":0,"window_id":"w"}"#,
            r#"{"egress":"internet","ttl_ms":900001,"window_id":"w"}"#,
            r#"{"egress":"internet","ttl_ms":"30000","window_id":"w"}"#,
            r#"{"egress":"internet","ttl_ms":1000}"#,
            r#"{"egress":"internet","ttl_ms":1000,"window_id":"has space"}"#,
            r#"{"egress":"internet","ttl_ms":1000,"window_id":"w","dns":"yes"}"#,
            r#"{"egress":"internet","ttl_ms":1000,"window_id":"w","extra":1}"#,
        ] {
            let error = open(body).err().unwrap_or_else(|| panic!("{body:?} was accepted"));
            assert_eq!(error.status, 400, "{body:?}");
        }
    }

    #[test]
    fn every_host_failure_has_its_own_stable_answer() {
        let answers: Vec<(u16, &str)> = [
            EgressSetFailure::NotFound,
            EgressSetFailure::NotWindowed,
            EgressSetFailure::NotActive,
            EgressSetFailure::FlipFailed,
            EgressSetFailure::Unavailable,
        ]
        .map(|failure| {
            let error = failure_error(failure);
            (error.status, error.code)
        })
        .to_vec();
        assert_eq!(
            answers,
            vec![
                (404, "NOT_FOUND"),
                (409, "egress_not_windowed"),
                (409, "egress_not_active"),
                (502, "egress_flip_failed"),
                (503, "egress_unavailable"),
            ]
        );
    }
}
